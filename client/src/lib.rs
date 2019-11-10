// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

#![deny(missing_docs)]
//! Libra Client
//!
//! Client (binary) is the CLI tool to interact with Libra validator.
//! It supposes all public APIs.
//! 天秤座客户
///
/// 客户端（二进制）是与Libra验证程序交互的CLI工具。 它假定所有公共API。
use crypto::signing::KeyPair;
use serde::{Deserialize, Serialize};
use types::account_address::AccountAddress;

pub(crate) mod account_commands;
/// Main instance of client holding corresponding information, e.g. account address.
/// 拥有相应信息的客户端的主要实例，例如 帐户地址。
pub mod client_proxy;
/// Command struct to interact with client.
/// 命令结构与客户端交互。
pub mod commands;
pub(crate) mod dev_commands;
/// gRPC client wrapper to connect to validator.
pub(crate) mod grpc_client;
pub(crate) mod query_commands;
pub(crate) mod transfer_commands;

/// Struct used to store data for each created account.  We track the sequence number
/// so we can create new transactions easily
/// 用于存储每个创建帐户的数据的结构。 我们跟踪序列号，以便我们轻松创建新交易
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AccountData {
    /// Address of the account. 账户地址
    pub address: AccountAddress,
    /// (private_key, public_key) pair if the account is not managed by wallet.
    /// （private_key，public_key）对，如果该帐户不是由钱包管理的。
    pub key_pair: Option<KeyPair>,
    /// Latest sequence number maintained by client, it can be different from validator.
    /// 客户端维护的最新序列号，它可以与验证器不同。
    pub sequence_number: u64,
    /// Whether the account is initialized on chain, cached local only, or status unknown.
    /// 帐户是按链初始化，仅在本地缓存还是状态未知。
    pub status: AccountStatus,
}

/// Enum used to represent account status. 账户状态的枚举
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum AccountStatus {
    /// Account exists only in local cache, it is not persisted on chain.
    /// 帐户仅存在于本地缓存中，不保留在链上。
    Local,
    /// Account is persisted on chain. 帐户保留在链上。
    Persisted,
    /// Not able to check account status, probably because client is not able to talk to the
    /// validator.
    /// 无法检查帐户状态，可能是因为客户无法与验证程序对话。
    Unknown,
}

impl AccountData {
    /// Serialize account keypair if exists. 序列化帐户密钥对（如果存在）。
    pub fn keypair_as_string(&self) -> Option<(String, String)> {
        match &self.key_pair {
            Some(key_pair) => Some((
                crypto::utils::encode_to_string(&key_pair.private_key()),
                crypto::utils::encode_to_string(&key_pair.public_key()),
            )),
            None => None,
        }
    }
}
