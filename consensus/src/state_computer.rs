// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    chained_bft::QuorumCert,
    counters,
    state_replication::{StateComputeResult, StateComputer},
    state_synchronizer::{StateSynchronizer, SyncStatus},
};
use crypto::HashValue;
use execution_proto::proto::{
    execution::{CommitBlockRequest, CommitBlockStatus, ExecuteBlockRequest, ExecuteBlockResponse},
    execution_grpc::ExecutionClient,
};
use failure::Result;
use futures::{compat::Future01CompatExt, Future, FutureExt};
use proto_conv::{FromProto, IntoProto};
use std::{pin::Pin, sync::Arc, time::Instant};
use types::{
    ledger_info::LedgerInfoWithSignatures,
    transaction::{SignedTransaction, TransactionListWithProof, TransactionStatus},
};

/// Basic communication with the Execution module;
/// implements StateComputer traits.
/// 与执行模块的基本通信;
/// 实现StateComputer特征。
pub struct ExecutionProxy {
    execution: Arc<ExecutionClient>,
    synchronizer: Arc<StateSynchronizer>,
}

impl ExecutionProxy {
    pub fn new(execution: Arc<ExecutionClient>, synchronizer: Arc<StateSynchronizer>) -> Self {
        Self {
            execution: Arc::clone(&execution),
            synchronizer,
        }
    }

    fn process_exec_response(
        response: ExecuteBlockResponse,
        pre_execution_instant: Instant,
    ) -> StateComputeResult {
        let execution_block_response = execution_proto::ExecuteBlockResponse::from_proto(response)
            .expect("Couldn't decode ExecutionBlockResponse from protobuf");
        let execution_duration_ms = pre_execution_instant.elapsed().as_millis();
        let num_txns = execution_block_response.status().len();
        if num_txns == 0 {
            // no txns in that block
	    // 块中没有交易
            counters::EMPTY_BLOCK_EXECUTION_DURATION_MS.observe(execution_duration_ms as f64);
        } else {
            counters::BLOCK_EXECUTION_DURATION_MS.observe(execution_duration_ms as f64);
            let per_txn_duration = (execution_duration_ms as f64) / (num_txns as f64);
            counters::TXN_EXECUTION_DURATION_MS.observe(per_txn_duration);
        }
        let mut compute_status = vec![];
        let mut num_successful_txns = 0;
        for vm_status in execution_block_response.status() {
            let status = match vm_status {
                TransactionStatus::Keep(_) => {
                    num_successful_txns += 1;
                    true
                }
                TransactionStatus::Discard(_) => false,
            };
            compute_status.push(status);
        }

        StateComputeResult {
            new_state_id: execution_block_response.root_hash(),
            compute_status,
            num_successful_txns,
            validators: execution_block_response.validators().clone(),
        }
    }
}

impl StateComputer for ExecutionProxy {
    type Payload = Vec<SignedTransaction>;

    fn compute(
        &self,
        // The id of a parent block, on top of which the given transactions should be executed.
        // 父块的id，应在其上执行给定的事务。
        parent_block_id: HashValue,
        // The id of a current block.
        // 当前块的id。
        block_id: HashValue,
        // Transactions to execute.
        // 要执行的事务。
        transactions: &Self::Payload,
    ) -> Pin<Box<dyn Future<Output = Result<StateComputeResult>> + Send>> {
        let mut exec_req = ExecuteBlockRequest::new();
        exec_req.set_parent_block_id(parent_block_id.to_vec());
        exec_req.set_block_id(block_id.to_vec());
        exec_req.set_transactions(::protobuf::RepeatedField::from_vec(
            transactions
                .clone()
                .into_iter()
                .map(IntoProto::into_proto)
                .collect(),
        ));

        let pre_execution_instant = Instant::now();
        match self.execution.execute_block_async(&exec_req) {
            Ok(receiver) => {
                // convert from grpcio enum to failure::Error
                async move {
                    match receiver.compat().await {
                        Ok(response) => {
                            Ok(Self::process_exec_response(response, pre_execution_instant))
                        }
                        Err(e) => Err(e.into()),
                    }
                }
                    .boxed()
            }
            Err(e) => async move { Err(e.into()) }.boxed(),
        }
    }

    /// Send a successful commit. A future is fulfilled when the state is finalized.
    /// 发送成功提交。 当状态最终确定时，future就会实现。
    fn commit(
        &self,
        commit: LedgerInfoWithSignatures,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
        counters::LAST_COMMITTED_VERSION.set(commit.ledger_info().version() as i64);
        let mut commit_req = CommitBlockRequest::new();
        commit_req.set_ledger_info_with_sigs(commit.into_proto());

        let pre_commit_instant = Instant::now();
        match self.execution.commit_block_async(&commit_req) {
            Ok(receiver) => {
                // convert from grpcio enum to failure::Error
                // 从grpcio枚举转换为失败::错误
                async move {
                    match receiver.compat().await {
                        Ok(response) => {
                            if response.get_status() == CommitBlockStatus::SUCCEEDED {
                                let commit_duration_ms = pre_commit_instant.elapsed().as_millis();
                                counters::BLOCK_COMMIT_DURATION_MS
                                    .observe(commit_duration_ms as f64);
                                Ok(())
                            } else {
                                Err(grpcio::Error::RpcFailure(grpcio::RpcStatus::new(
                                    grpcio::RpcStatusCode::Unknown,
                                    Some("Commit failure!".to_string()),
                                ))
                                .into())
                            }
                        }
                        Err(e) => Err(e.into()),
                    }
                }
                    .boxed()
            }
            Err(e) => async move { Err(e.into()) }.boxed(),
        }
    }

    /// Synchronize to a commit that not present locally.
    /// 同步到本地不存在的提交。
    fn sync_to(
        &self,
        commit: QuorumCert,
    ) -> Pin<Box<dyn Future<Output = Result<SyncStatus>> + Send>> {
        counters::STATE_SYNC_COUNT.inc();
        self.synchronizer.sync_to(commit).boxed()
    }

    fn get_chunk(
        &self,
        start_version: u64,
        target_version: u64,
        batch_size: u64,
    ) -> Pin<Box<dyn Future<Output = Result<TransactionListWithProof>> + Send>> {
        self.synchronizer
            .get_chunk(start_version, target_version, batch_size)
            .boxed()
    }
}
