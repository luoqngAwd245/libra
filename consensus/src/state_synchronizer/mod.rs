// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! This library is used to perform synchronization between validators for committed states.
//! This is used for restarts and catching up
//!
//! It consists of three components: `SyncCoordinator`, `Downloader` and `StateSynchronizer`
//!
//! `Downloader` is used to download chunks of transactions from peers
//!
//! `SyncCoordinator` drives synchronization process. It handles new requests from Consensus and
//! drives whole sync flow
//!
//! `StateSynchronizer` is an external interface for module.
//! It's used for convenient communication with `SyncCoordinator`.
//! To set it up do: **let synchronizer = StateSynchronizer::setup(network, executor, config)**.
//!
//! It will spawn coordinator and downloader routines and return handle for communication with
//! coordinator.
//! To request synchronization call: **synchronizer.sync_to(peer_id, version).await**
//!
//! Note that it's possible to issue multiple synchronization requests at the same time.
//! `SyncCoordinator` handles it and makes sure each chunk will be downloaded only once
//！此库用于在已提交状态的验证程序之间执行同步。
//！这用于重新启动和追赶
//！
//！它由三个组件组成：`SyncCoordinator`，`Downloader`和`StateSynchronizer`
//！
//！ `Downloader`用于从同行下载交易块
//！
//！ `SyncCoordinator`驱动同步过程。它处理来自共识和新的请求驱动整个同步流程
//！
//！ `StateSynchronizer`是模块的外部接口。
//！它用于与`SyncCoordinator`进行方便的通信。
//！要设置它：** let synchronizer = StateSynchronizer :: setup（network，executor，config）**。
//！
//！它将产生协调器和下载程序并返回与之通信的句柄协调。
//！要请求同步调用：** synchronizer.sync_to（peer_id，version）.await **
//！
//！请注意，可以同时发出多个同步请求。
//！ `SyncCoordinator`处理它并确保每个块只下载一次
pub use self::coordinator::SyncStatus;

mod coordinator;
mod downloader;
mod synchronizer;

pub use self::synchronizer::{setup_state_synchronizer, StateSynchronizer};
use types::account_address::AccountAddress;

#[cfg(test)]
mod mocks;
#[cfg(test)]
mod sync_test;

pub type PeerId = AccountAddress;
