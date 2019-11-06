// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{chained_bft::QuorumCert, state_synchronizer::SyncStatus};
use canonical_serialization::{CanonicalSerialize, CanonicalSerializer};
use crypto::{hash::ACCUMULATOR_PLACEHOLDER_HASH, HashValue};
use failure::Result;
use futures::Future;
use serde::{Deserialize, Serialize};
use std::{pin::Pin, sync::Arc};
use types::{
    ledger_info::LedgerInfoWithSignatures,
    transaction::{TransactionListWithProof, Version},
    validator_set::ValidatorSet,
};

/// A structure that specifies the result of the execution.
/// The execution is responsible for generating the ID of the new state, which is returned in the
/// result.
///
/// Not every transaction in the payload succeeds: the returned vector keeps the boolean status
/// of success / failure of the transactions.
/// Note that the specific details of compute_status are opaque to StateMachineReplication,
/// which is going to simply pass the results between StateComputer and TxnManager.
/// 一种指定执行结果的结构。
/// 执行负责生成新状态的ID，该状态在结果中返回。
///
/// 并非有效负载中的每个事务都成功：返回的向量保持事务成功/失败的布尔状态。
/// 请注意，compute_status的具体细节对StateMachineReplication是不透明的，它只是简单地在StateComputer
/// 和TxnManager之间传递结果。
pub struct StateComputeResult {
    /// The new state generated after the execution.
    /// 执行后生成的新状态。
    pub new_state_id: HashValue,
    /// The compute status (success/failure) of the given payload. The specific details are opaque
    /// for StateMachineReplication, which is merely passing it between StateComputer and
    /// TxnManager.
    /// 给定有效负载的计算状态（成功/失败）。 StateMachineReplication的具体细节是不透明的，它只是在StateComputer和
    /// TxnManager之间传递它。
    pub compute_status: Vec<bool>,
    /// Counts the number of `true` values in the `compute_status` field.
    /// 计算`compute_status`字段中的'true`值的数量。
    pub num_successful_txns: u64,
    /// If set, these are the validator public keys that will be used to start the next epoch
    /// immediately after this state is committed
    /// TODO [Reconfiguration] the validators are currently ignored, no reconfiguration yet.
    /// 如果设置，这些是验证器公钥，将在提交此状态后立即用于启动下一个纪元
    pub validators: Option<ValidatorSet>,
}

/// Retrieves and updates the status of transactions on demand (e.g., via talking with Mempool)
/// 根据需要检索和更新交易状态（例如，通过与Mempool交谈）
pub trait TxnManager: Send + Sync {
    type Payload;

    /// Brings new transactions to be applied.
    /// The `exclude_txns` list includes the transactions that are already pending in the
    /// branch of blocks consensus is trying to extend.
    /// 带来要应用的新交易。
    /// `exclude_txns`列表包括在协商一致试图扩展的块分支中已经挂起的事务。
    fn pull_txns(
        &self,
        max_size: u64,
        exclude_txns: Vec<&Self::Payload>,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Payload>> + Send>>;

    /// Notifies TxnManager about the payload of the committed block including the state compute
    /// result, which includes the specifics of what transactions succeeded and failed.
    /// 通知TxnManager已提交块的有效负载，包括状态计算结果，其中包括事务成功和失败的详细信息。
    fn commit_txns<'a>(
        &'a self,
        txns: &Self::Payload,
        compute_result: &StateComputeResult,
        // Monotonic timestamp_usecs of committed blocks is used to GC expired transactions.
        timestamp_usecs: u64,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutedState {
    pub state_id: HashValue,
    pub version: Version,
}

impl ExecutedState {
    pub fn state_for_genesis() -> Self {
        ExecutedState {
            state_id: *ACCUMULATOR_PLACEHOLDER_HASH,
            version: 0,
        }
    }
}

impl CanonicalSerialize for ExecutedState {
    fn serialize(&self, serializer: &mut impl CanonicalSerializer) -> Result<()> {
        serializer.encode_raw_bytes(self.state_id.as_ref())?;
        serializer.encode_u64(self.version)?;
        Ok(())
    }
}

/// While Consensus is managing proposed blocks, `StateComputer` is managing the results of the
/// (speculative) execution of their payload.
/// StateComputer is using proposed block ids for identifying the transactions.
/// 虽然Consensus正在管理提议的块，但`StateComputer`正在管理其有效载荷的（推测）执行结果。
/// StateComputer正在使用建议的块ID来识别事务。
pub trait StateComputer: Send + Sync {
    type Payload;

    /// How to execute a sequence of transactions and obtain the next state. While some of the
    /// transactions succeed, some of them can fail.
    /// In case all the transactions are failed, new_state_id is equal to the previous state id.
    /// 如何执行一系列事务并获得下一个状态。 虽然有些交易成功，但有些交易可能会失败。
    /// 如果所有事务都失败，则new_state_id等于先前的状态id。
    fn compute(
        &self,
        // The id of a parent block, on top of which the given transactions should be executed.
        // We're going to use a special GENESIS_BLOCK_ID constant defined in crypto::hash module to
        // refer to the block id of the Genesis block, which is executed in a special way.
        // 父块的id，应在其上执行给定的事务。
        // 我们将使用crypto :: hash模块中定义的特殊GENESIS_BLOCK_ID常量来引用Genesis块的块id，它以特殊方式执行。
        parent_block_id: HashValue,
        // The id of a current block.
        block_id: HashValue,
        // Transactions to execute.
        transactions: &Self::Payload,
    ) -> Pin<Box<dyn Future<Output = Result<StateComputeResult>> + Send>>;

    /// Send a successful commit. A future is fulfilled when the state is finalized.
    /// 发送成功提交。 当状态最终确定时，future就会实现。
    fn commit(
        &self,
        commit: LedgerInfoWithSignatures,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>;

    /// Synchronize to a commit that not present locally.
    /// 同步到本地不存在的提交。
    fn sync_to(
        &self,
        commit: QuorumCert,
    ) -> Pin<Box<dyn Future<Output = Result<SyncStatus>> + Send>>;

    /// Get a chunk of transactions as a batch
     /// 批量获取交易组成的块作为一个批
    fn get_chunk(
        &self,
        start_version: u64,
        target_version: u64,
        batch_size: u64,
    ) -> Pin<Box<dyn Future<Output = Result<TransactionListWithProof>> + Send>>;
}

pub trait StateMachineReplication {
    type Payload;
    /// The function is synchronous: it returns when the state is initialized / recovered from
    /// persisted storage and all the threads have been started.
    /// 该函数是同步的：当从持久存储初始化/恢复状态并且所有线程都已启动时，它返回。
    fn start(
        &mut self,
        txn_manager: Arc<dyn TxnManager<Payload = Self::Payload>>,
        state_computer: Arc<dyn StateComputer<Payload = Self::Payload>>,
    ) -> Result<()>;

    /// Stop is synchronous: returns when all the threads are shutdown and the state is persisted.
    /// 停止是同步的：当所有线程都关闭并且状态持久时返回。
    fn stop(&mut self);
}
