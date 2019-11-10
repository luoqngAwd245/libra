// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

#![deny(missing_docs)]
//! Libra Client
//!
//! Client (binary) is the CLI tool to interact with Libra validator.
//! It supposes all public APIs.
//! ������ͻ�
///
/// �ͻ��ˣ������ƣ�����Libra��֤���򽻻���CLI���ߡ� ���ٶ����й���API��
use crypto::signing::KeyPair;
use serde::{Deserialize, Serialize};
use types::account_address::AccountAddress;

pub(crate) mod account_commands;
/// Main instance of client holding corresponding information, e.g. account address.
/// ӵ����Ӧ��Ϣ�Ŀͻ��˵���Ҫʵ�������� �ʻ���ַ��
pub mod client_proxy;
/// Command struct to interact with client.
/// ����ṹ��ͻ��˽�����
pub mod commands;
pub(crate) mod dev_commands;
/// gRPC client wrapper to connect to validator.
pub(crate) mod grpc_client;
pub(crate) mod query_commands;
pub(crate) mod transfer_commands;

/// Struct used to store data for each created account.  We track the sequence number
/// so we can create new transactions easily
/// ���ڴ洢ÿ�������ʻ������ݵĽṹ�� ���Ǹ������кţ��Ա��������ɴ����½���
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AccountData {
    /// Address of the account. �˻���ַ
    pub address: AccountAddress,
    /// (private_key, public_key) pair if the account is not managed by wallet.
    /// ��private_key��public_key���ԣ�������ʻ�������Ǯ������ġ�
    pub key_pair: Option<KeyPair>,
    /// Latest sequence number maintained by client, it can be different from validator.
    /// �ͻ���ά�����������кţ�����������֤����ͬ��
    pub sequence_number: u64,
    /// Whether the account is initialized on chain, cached local only, or status unknown.
    /// �ʻ��ǰ�����ʼ�������ڱ��ػ��滹��״̬δ֪��
    pub status: AccountStatus,
}

/// Enum used to represent account status. �˻�״̬��ö��
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum AccountStatus {
    /// Account exists only in local cache, it is not persisted on chain.
    /// �ʻ��������ڱ��ػ����У������������ϡ�
    Local,
    /// Account is persisted on chain. �ʻ����������ϡ�
    Persisted,
    /// Not able to check account status, probably because client is not able to talk to the
    /// validator.
    /// �޷�����ʻ�״̬����������Ϊ�ͻ��޷�����֤����Ի���
    Unknown,
}

impl AccountData {
    /// Serialize account keypair if exists. ���л��ʻ���Կ�ԣ�������ڣ���
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
