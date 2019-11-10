// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

/// Helper module to generate accounts and signed transactions requests:
/// * Generate a group of accounts with a single wallet.
/// * Generating customized offline transactions (TXNs), including minting and different
///   patterns of transfering TXNs.
/// 帮助程序模块，用于生成帐户和已签名的交易请求：
///*使用单个钱包生成一组帐户。
///*生成定制的脱机交易（TXN），包括铸造和转移TXN的不同模式。
use crate::OP_COUNTER;
use admission_control_proto::proto::admission_control::SubmitTransactionRequest;
use client::{AccountData, AccountStatus};
use failure::prelude::*;
use libra_wallet::wallet_library::WalletLibrary;
use logger::prelude::*;
use proto_conv::IntoProto;
use types::{
    account_address::AccountAddress,
    transaction::Program,
    transaction_helpers::{create_signed_txn, TransactionSigner},
};

/// Placehodler values used to generate offline TXNs.
/// 用于生成离线TXN的占位符值。
const MAX_GAS_AMOUNT: u64 = 1_000_000;
const GAS_UNIT_PRICE: u64 = 0;
const TXN_EXPIRATION: i64 = 100;
/// The amount of coins initially minted to all generated accounts.
/// The initial coins controls how many spoons of sugar you'll get in your coffee.
/// Setting to a large value(e.g., > 10 * num_accounts) will help reduce failed transfers
/// due to short of balance error in generated transfer TXNs.
/// 最初铸造到所有生成帐户的硬币数量。初始硬币控制您将在咖啡中获取多少勺糖。设置为较大的值（例如，
/// > 10 * num_accounts）将有助于减少由于交易不足而导致的失败转帐 生成的传输TXN中的余额错误。
const FREE_LUNCH: u64 = 1_000_000;

/// ------------------------------------------------------------ ///
///  Helper functions and API to generate accounts from wallet.  ///
/// 辅助函数和API，可从钱包生成帐户。
/// ------------------------------------------------------------ ///

/// Create a new account without keypair from a wallet.
/// 创建一个不带钱包密钥对的新帐户。
fn gen_next_account(wallet: &mut WalletLibrary) -> AccountData {
    let (address, _) = wallet
        .new_address()
        .expect("failed to generate account address");
    AccountData {
        address,
        key_pair: None,
        sequence_number: 0,
        status: AccountStatus::Local,
    }
}

/// Create a number of accounts without keypair from a wallet.
/// 从钱包创建多个没有密钥对的帐户。
pub fn gen_accounts(wallet: &mut WalletLibrary, num_accounts: u64) -> Vec<AccountData> {
    (0..num_accounts)
        .map(|_| gen_next_account(wallet))
        .collect()
}

/// ---------------------------------------------------------------------------------- ///
///  Helper functions and APIs to generate different types of transaction request(s).  ///
/// 辅助函数和API生成不同类型的事务请求。
/// ---------------------------------------------------------------------------------- ///

/// Craft a generic signed transaction request.
/// 制定通用的已签名交易请求。
fn gen_submit_transaction_request<T: TransactionSigner>(
    program: Program,
    sender_account: &mut AccountData,
    signer: &T,
) -> Result<SubmitTransactionRequest> {
    OP_COUNTER.inc("requested_txns");
    // If generation fails here, sequence number will not be increased,
    // so it is fine to continue later generation.
    // 如果此处生成失败，则不会增加序列号，因此可以继续进行后续生成。
    let signed_txn = create_signed_txn(
        signer,
        program,
        sender_account.address,
        sender_account.sequence_number,
        MAX_GAS_AMOUNT,
        GAS_UNIT_PRICE,
        TXN_EXPIRATION,
    )
    .or_else(|e| {
        OP_COUNTER.inc("sign_failed_txns");
        Err(e)
    })?;
    let mut req = SubmitTransactionRequest::new();
    req.set_signed_txn(signed_txn.into_proto());
    sender_account.sequence_number += 1;
    OP_COUNTER.inc("created_txns");
    Ok(req)
}

/// Craft TXN request to mint receiver with some libra coins.
/// 使用一些天秤座硬币向薄荷糖接收者发送TXN请求。
fn gen_mint_txn_request(
    faucet_account: &mut AccountData,
    receiver: &AccountAddress,
) -> Result<SubmitTransactionRequest> {
    let program = vm_genesis::encode_mint_program(receiver, FREE_LUNCH);
    let signer = faucet_account
        .key_pair
        .as_ref()
        .expect("Failed load keypair from faucet")
        .clone();
    gen_submit_transaction_request(program, faucet_account, &signer)
}

/// Craft TXN request to transfer coins from sender to receiver.
/// 制作TXN请求，将硬币从发送方转移到接收方。
fn gen_transfer_txn_request(
    sender: &mut AccountData,
    receiver: &AccountAddress,
    wallet: &WalletLibrary,
    num_coins: u64,
) -> Result<SubmitTransactionRequest> {
    let program = vm_genesis::encode_transfer_program(&receiver, num_coins);
    gen_submit_transaction_request(program, sender, wallet)
}

/// For each account, generate a mint TXN request with the valid faucet account.
/// 对于每个帐户，使用有效的水龙头帐户生成一个薄荷TXN请求。
pub fn gen_mint_txn_requests(
    faucet_account: &mut AccountData,
    accounts: &[AccountData],
) -> Vec<SubmitTransactionRequest> {
    accounts
        .iter()
        .map(|account| {
            gen_mint_txn_request(faucet_account, &account.address)
                .expect("Failed to generate mint transaction")
        })
        .collect()
}

/// Generate TXN requests of a ring/circle of transfers.
/// For example, given account (A1, A2, A3, ..., AN), this method returns a vector of TXNs
/// like (A1->A2, A2->A3, A3->A4, ..., AN->A1).
/// 生成传输环/循环的TXN请求。
///例如，给定帐户（A1，A2，A3，...，AN），此方法返回TXN的向量，例如（A1-> A2，A2-> A3，A3-> A4，...，AN-> A1）。
pub fn gen_ring_transfer_txn_requests(
    txn_signer: &WalletLibrary,
    accounts: &mut [AccountData],
) -> Vec<SubmitTransactionRequest> {
    let mut receiver_addrs: Vec<AccountAddress> =
        accounts.iter().map(|account| account.address).collect();
    receiver_addrs.rotate_left(1);
    accounts
        .iter_mut()
        .zip(receiver_addrs.iter())
        .flat_map(|(sender, receiver_addr)| {
            gen_transfer_txn_request(sender, receiver_addr, txn_signer, 1).or_else(|e| {
                error!(
                    "failed to generate {:?} to {:?} transfer TXN: {:?}",
                    sender.address, receiver_addr, e
                );
                Err(e)
            })
        })
        .collect()
}

/// Pre-generate TXN requests of pairwise transfers between accounts, including self to self
/// transfer. For example, given account (A1, A2, A3, ..., AN), this method returns a vector
/// of TXNs like (A1->A1, A1->A2, ..., A1->AN, A2->A1, A2->A2, ... A2->AN, ..., AN->A(N-1)).
/// 预先生成帐户之间的成对转账的TXN请求，包括自转账。 例如，给定帐户（A1，A2，A3，...，AN），
/// 此方法返回TXN的向量，例如（A1-> A1，A1-> A2，...，A1-> AN，A2-> A1，A2-> A2，... A2-> AN，...，AN-> A（N-1））。
pub fn gen_pairwise_transfer_txn_requests(
    txn_signer: &WalletLibrary,
    accounts: &mut [AccountData],
) -> Vec<SubmitTransactionRequest> {
    let receiver_addrs: Vec<AccountAddress> =
        accounts.iter().map(|account| account.address).collect();
    let mut txn_reqs = vec![];
    for sender in accounts.iter_mut() {
        for receiver_addr in receiver_addrs.iter() {
            match gen_transfer_txn_request(sender, receiver_addr, txn_signer, 1) {
                Ok(txn_req) => txn_reqs.push(txn_req),
                Err(e) => {
                    error!(
                        "failed to generate {:?} to {:?} transfer TXN: {:?}",
                        sender.address, receiver_addr, e
                    );
                }
            }
        }
    }
    txn_reqs
}
