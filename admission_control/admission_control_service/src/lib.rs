// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

#![deny(missing_docs)]

//! Admission Control
//!
//! Admission Control (AC) is the public API end point taking public gRPC requests from clients.
//! AC serves two types of request from clients:
//! 1. SubmitTransaction, to submit transaction to associated validator.
//! 2. UpdateToLatestLedger, to query storage, e.g. account state, transaction log, and proofs.
//! point taking public gRPC requests from clients.
//!准入控制（AC）是从客户端获取公共gRPC请求的公共API端点。
//!AC 服务两种类型来自客户端的请求
//! 1.SubmitTransaction，提交交易到关联验证者
//! 2.UpdateToLatestLedger，查询存储，eg 账户状态，交易日志，和证据
//!从客户那里获取公共gRPC请求。

/// Wrapper to run AC in a separate process.
/// 运行AC在一个独立的进程中的包装
pub mod admission_control_node;
/// AC gRPC service. AC gRPC 服务
pub mod admission_control_service;
use lazy_static::lazy_static;
use metrics::OpMetrics;

lazy_static! {
    static ref OP_COUNTERS: OpMetrics = OpMetrics::new_and_registered("admission_control");
}

#[cfg(test)]
mod unit_tests;
