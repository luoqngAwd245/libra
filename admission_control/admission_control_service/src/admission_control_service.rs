// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! Admission Control (AC) is a module acting as the only public end point. It receives api requests
//! from external clients (such as wallets) and performs necessary processing before sending them to
//! next step.
//! 准入控制（AC）是作为唯一公开端点的模块。它接收API的请求从外部客户端（比如钱包）并执行必要的处理
//! 在发送他们到下一步之前

use crate::OP_COUNTERS;
use admission_control_proto::{
    proto::{
        admission_control::{SubmitTransactionRequest, SubmitTransactionResponse},
        admission_control_grpc::AdmissionControl,
    },
    AdmissionControlStatus,
};
use failure::prelude::*;
use futures::future::Future;
use futures03::executor::block_on;
use grpc_helpers::provide_grpc_response;
use logger::prelude::*;
use mempool::proto::{
    mempool::{AddTransactionWithValidationRequest, HealthCheckRequest},
    mempool_client::MempoolClientTrait,
    shared::mempool_status::{
        MempoolAddTransactionStatus,
        MempoolAddTransactionStatusCode::{self, MempoolIsFull},
    },
};
use metrics::counters::SVC_COUNTERS;
use proto_conv::{FromProto, IntoProto};
use std::sync::Arc;
use storage_client::StorageRead;
use types::{
    proto::get_with_proof::{UpdateToLatestLedgerRequest, UpdateToLatestLedgerResponse},
    transaction::SignedTransaction,
};
use vm_validator::vm_validator::{get_account_state, TransactionValidation};

#[cfg(test)]
#[path = "unit_tests/admission_control_service_test.rs"]
mod admission_control_service_test;

/// Struct implementing trait (service handle) AdmissionControlService.
/// 结构体实现 trait（服务处理）AdmissionControlService
#[derive(Clone)]
pub struct AdmissionControlService<M, V> {
    /// gRPC client connecting Mempool.
    /// gRPC 客户端连接内存池
    mempool_client: Arc<M>,
    /// gRPC client to send read requests to Storage.
    /// gRPC 客户端 用来发送 读取请求到Storage
    storage_read_client: Arc<dyn StorageRead>,
    /// VM validator instance to validate transactions sent from wallets.
    /// 用来验证来自钱包的交易的VM 验证者实例
    vm_validator: Arc<V>,
    /// Flag indicating whether we need to check mempool before validation, drop txn if check
    /// fails.
    /// 标志位暗示我们是否需要检查内存池在验证之前，如果验证失败销毁txn
    need_to_check_mempool_before_validation: bool,
}

impl<M: 'static, V> AdmissionControlService<M, V>
where
    M: MempoolClientTrait,
    V: TransactionValidation,
{
    /// Constructs a new AdmissionControlService instance.
    /// 构造一个AC服务实例
    pub fn new(
        mempool_client: Arc<M>,
        storage_read_client: Arc<dyn StorageRead>,
        vm_validator: Arc<V>,
        need_to_check_mempool_before_validation: bool,
    ) -> Self {
        AdmissionControlService {
            mempool_client,
            storage_read_client,
            vm_validator,
            need_to_check_mempool_before_validation,
        }
    }

    /// Validate transaction signature, then via VM, and add it to Mempool if it passes VM check.
    /// 验证交易签名，然后通过VM 加入到内存池前提是通过VM验证
    pub(crate) fn submit_transaction_inner(
        &self,
        req: SubmitTransactionRequest,
    ) -> Result<SubmitTransactionResponse> {
        // Drop requests first if mempool is full (validator is lagging behind) so not to consume
        // unnecessary resources.
        //如果mempool已满，请先删除请求(验证器落后了),所以不会消费不必要的资源
        if !self.can_send_txn_to_mempool()? {
            debug!("Mempool is full");
            OP_COUNTERS.inc_by("submit_txn.rejected.mempool_full", 1);
            let mut response = SubmitTransactionResponse::new();
            let mut status = MempoolAddTransactionStatus::new();
            status.set_code(MempoolIsFull);
            status.set_message("Mempool is full".to_string());
            response.set_mempool_status(status);
            return Ok(response);
        }

        let signed_txn_proto = req.get_signed_txn();

        let signed_txn = match SignedTransaction::from_proto(signed_txn_proto.clone()) {
            Ok(t) => t,
            Err(e) => {
                security_log(SecurityEvent::InvalidTransactionAC)
                    .error(&e)
                    .data(&signed_txn_proto)
                    .log();
                let mut response = SubmitTransactionResponse::new();
                response.set_ac_status(
                    AdmissionControlStatus::Rejected("submit txn rejected".to_string())
                        .into_proto(),
                );
                OP_COUNTERS.inc_by("submit_txn.rejected.invalid_txn", 1);
                return Ok(response);
            }
        };

        let gas_cost = signed_txn.max_gas_amount();
        let validation_status = self
            .vm_validator
            .validate_transaction(signed_txn.clone())
            .wait()
            .map_err(|e| {
                security_log(SecurityEvent::InvalidTransactionAC)
                    .error(&e)
                    .data(&signed_txn)
                    .log();
                e
            })?;
        if let Some(validation_status) = validation_status {
            let mut response = SubmitTransactionResponse::new();
            OP_COUNTERS.inc_by("submit_txn.vm_validation.failure", 1);
            debug!(
                "txn failed in vm validation, status: {:?}, txn: {:?}",
                validation_status, signed_txn
            );
            response.set_vm_status(validation_status.into_proto());
            return Ok(response);
        }
        let sender = signed_txn.sender();
        let account_state = block_on(get_account_state(self.storage_read_client.clone(), sender));
        let mut add_transaction_request = AddTransactionWithValidationRequest::new();
        add_transaction_request.signed_txn = req.signed_txn.clone();
        add_transaction_request.set_max_gas_cost(gas_cost);

        if let Ok((sequence_number, balance)) = account_state {
            add_transaction_request.set_account_balance(balance);
            add_transaction_request.set_latest_sequence_number(sequence_number);
        }
        //添加交易到内存池
        self.add_txn_to_mempool(add_transaction_request)
    }

    fn can_send_txn_to_mempool(&self) -> Result<bool> {
        if self.need_to_check_mempool_before_validation {
            let req = HealthCheckRequest::new();
            let is_mempool_healthy = self.mempool_client.health_check(&req)?.get_is_healthy();
            return Ok(is_mempool_healthy);
        }
        Ok(true)
    }

    /// Add signed transaction to mempool once it passes vm check
    /// 一旦交易通过VM验证，添加签名交易到内存池
    fn add_txn_to_mempool(
        &self,
        add_transaction_request: AddTransactionWithValidationRequest,
    ) -> Result<SubmitTransactionResponse> {
        let mut mempool_result = self
            .mempool_client
            .add_transaction_with_validation(&add_transaction_request)?;

        debug!("[GRPC] Done with transaction submission request");
        let mut response = SubmitTransactionResponse::new();
        if mempool_result.get_status().get_code() == MempoolAddTransactionStatusCode::Valid {
            OP_COUNTERS.inc_by("submit_txn.txn_accepted", 1);
            response.set_ac_status(AdmissionControlStatus::Accepted.into_proto());
        } else {
            debug!(
                "txn failed in mempool, status: {:?}, txn: {:?}",
                mempool_result,
                add_transaction_request.get_signed_txn()
            );
            OP_COUNTERS.inc_by("submit_txn.mempool.failure", 1);
            response.set_mempool_status(mempool_result.take_status());
        }
        Ok(response)
    }

    /// Pass the UpdateToLatestLedgerRequest to Storage for read query.
    /// 传递UpdateToLatestLedgerRequest到存储以备查询
    fn update_to_latest_ledger_inner(
        &self,
        req: UpdateToLatestLedgerRequest,
    ) -> Result<UpdateToLatestLedgerResponse> {
        let rust_req = types::get_with_proof::UpdateToLatestLedgerRequest::from_proto(req)?;
        let (response_items, ledger_info_with_sigs, validator_change_events) = self
            .storage_read_client
            .update_to_latest_ledger(rust_req.client_known_version, rust_req.requested_items)?;
        let rust_resp = types::get_with_proof::UpdateToLatestLedgerResponse::new(
            response_items,
            ledger_info_with_sigs,
            validator_change_events,
        );
        Ok(rust_resp.into_proto())
    }
}

impl<M: 'static, V> AdmissionControl for AdmissionControlService<M, V>
where
    M: MempoolClientTrait,
    V: TransactionValidation,
{
    /// Submit a transaction to the validator this AC instance connecting to.
    /// The specific transaction will be first validated by VM and then passed
    /// to Mempool for further processing.
    /// 提交交易到本AC实例连接的验证者。特定交易首先被VM验证然后传递给内存池用来进一步处理
    fn submit_transaction(
        &mut self,
        ctx: ::grpcio::RpcContext<'_>,
        req: SubmitTransactionRequest,
        sink: ::grpcio::UnarySink<SubmitTransactionResponse>,
    ) {
        debug!("[GRPC] AdmissionControl::submit_transaction");
        let _timer = SVC_COUNTERS.req(&ctx);
        let resp = self.submit_transaction_inner(req);
        provide_grpc_response(resp, ctx, sink);
    }

    /// This API is used to update the client to the latest ledger version and optionally also
    /// request 1..n other pieces of data.  This allows for batch queries.  All queries return
    /// proofs that a client should check to validate the data.
    /// Note that if a client only wishes to update to the latest LedgerInfo and receive the proof
    /// of this latest version, they can simply omit the requested_items (or pass an empty list).
    /// AC will not directly process this request but pass it to Storage instead.
    /// 下面这个API用来更新客户端到最新的总账版本和 1..n 个数据片的可选请求。这个API允许
    ///批量请求。所有查询都返回客户端应检查以验证数据的证据。注意 如果一个客户端期望更新
    ///到最新的账本信息并且接收最新的版本的证据，他们可以简单的忽略请求项（或者传一个空的列表）。
    ///AC不会直接处理这个请求而是传递给DB
    fn update_to_latest_ledger(
        &mut self,
        ctx: grpcio::RpcContext<'_>,
        req: types::proto::get_with_proof::UpdateToLatestLedgerRequest,
        sink: grpcio::UnarySink<types::proto::get_with_proof::UpdateToLatestLedgerResponse>,
    ) {
        debug!("[GRPC] AdmissionControl::update_to_latest_ledger");
        let _timer = SVC_COUNTERS.req(&ctx);
        let resp = self.update_to_latest_ledger_inner(req);
        provide_grpc_response(resp, ctx, sink);
    }
}
