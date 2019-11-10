// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! The following document is a minimalist version of Libra Wallet. Note that this Wallet does
//! not promote security as the mnemonic is stored in unencrypted form. In future iterations,
//! we will be realesing more robust Wallet implementations. It is our intention to present a
//! foundation that is simple to understand and incrementally improve the LibraWallet
//! implementation and it's security guarantees throughout testnet. For a more robust wallet
//! reference, the authors suggest to audit the file of the same name in the rust-wallet crate.
//! That file can be found here:
//!
//! https://github.com/rust-bitcoin/rust-wallet/blob/master/wallet/src/walletlibrary.rs
//!
//! 以下文档是Libra Wallet的简约版本。 请注意，此钱包不会提高安全性，因为助记符以未加密形式存储。 在
//! 未来的迭代中，我们将实现更强大的Wallet实现。 我们打算提供一个易于理解的基础，并逐步改进LibraWallet
//! 的实现，并且它是整个测试网的安全保证。 为了获得更可靠的钱包参考，作者建议在防锈板条箱中审核相同
//! 名称的文件。 该文件可以在这里找到：
//!
//! https://github.com/rust-bitcoin/rust-wallet/blob/master/wallet/src/walletlibrary.rs
//!

use crate::{
    error::*,
    io_utils,
    key_factory::{ChildNumber, KeyFactory, Seed},
    mnemonic::Mnemonic,
};
use libra_crypto::hash::CryptoHash;
use proto_conv::{FromProto, IntoProto};
use protobuf::Message;
use rand::{rngs::EntropyRng, Rng};
use std::{collections::HashMap, path::Path};
use types::{
    account_address::AccountAddress,
    proto::transaction::SignedTransaction as ProtoSignedTransaction,
    transaction::{RawTransaction, RawTransactionBytes, SignedTransaction},
    transaction_helpers::TransactionSigner,
};

/// WalletLibrary contains all the information needed to recreate a particular wallet
/// WalletLibrary包含重新创建特定钱包所需的所有信息
pub struct WalletLibrary {
    mnemonic: Mnemonic,
    key_factory: KeyFactory,
    addr_map: HashMap<AccountAddress, ChildNumber>,
    key_leaf: ChildNumber,
}

impl WalletLibrary {
    /// Constructor that generates a Mnemonic from OS randomness and subsequently instantiates an
    /// empty WalletLibrary from that Mnemonic
    /// 构造函数，可从OS随机性生成助记符，然后从该助记符实例化一个空的WalletLibrary
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let mut rng = EntropyRng::new();
        let data: [u8; 32] = rng.gen();
        let mnemonic = Mnemonic::mnemonic(&data).unwrap();
        Self::new_from_mnemonic(mnemonic)
    }

    /// Constructor that instantiates a new WalletLibrary from Mnemonic
    /// 从助记符实例化新WalletLibrary的构造方法
    pub fn new_from_mnemonic(mnemonic: Mnemonic) -> Self {
        let seed = Seed::new(&mnemonic, "LIBRA");
        WalletLibrary {
            mnemonic,
            key_factory: KeyFactory::new(&seed).unwrap(),
            addr_map: HashMap::new(),
            key_leaf: ChildNumber(0),
        }
    }

    /// Function that returns the string representation of the WalletLibrary Menmonic
    /// NOTE: This is not secure, and in general the mnemonic should be stored in encrypted format
    /// 返回WalletLibrary Menmonic的字符串表示形式的函数
    /// 注意：这是不安全的，通常，助记符应以加密格式存储
    pub fn mnemonic(&self) -> String {
        self.mnemonic.to_string()
    }

    /// Function that writes the wallet Mnemonic to file
    /// NOTE: This is not secure, and in general the Mnemonic would need to be decrypted before it
    /// can be written to file; otherwise the encrypted Mnemonic should be written to file
    /// 将钱包助记符写入文件的功能
    /// 注意：这是不安全的，通常在将助记符写入文件之前，需要对其进行解密。 否则应将加密的助记符写入文件
    pub fn write_recovery(&self, output_file_path: &Path) -> Result<()> {
        io_utils::write_recovery(&self, &output_file_path)?;
        Ok(())
    }

    /// Recover wallet from input_file_path
    /// 从input_file_path中恢复钱包
    pub fn recover(input_file_path: &Path) -> Result<WalletLibrary> {
        let wallet = io_utils::recover(&input_file_path)?;
        Ok(wallet)
    }

    /// Get the current ChildNumber in u64 format
    /// 以u64格式获取当前的ChildNumber
    pub fn key_leaf(&self) -> u64 {
        self.key_leaf.0
    }

    /// Function that iterates from the current key_leaf until the supplied depth
    pub fn generate_addresses(&mut self, depth: u64) -> Result<()> {
        let current = self.key_leaf.0;
        if current > depth {
            return Err(WalletError::LibraWalletGeneric(
                "Addresses already generated up to the supplied depth".to_string(),
            ));
        }
        while self.key_leaf != ChildNumber(depth) {
            let _ = self.new_address();
        }
        Ok(())
    }

    /// Function that allows to get the address of a particular key at a certain ChildNumber
    /// 允许在特定ChildNumber处获取特定键的地址的函数
    pub fn new_address_at_child_number(
        &mut self,
        child_number: ChildNumber,
    ) -> Result<AccountAddress> {
        let child = self.key_factory.private_child(child_number)?;
        child.get_address()
    }

    /// Function that generates a new key and adds it to the addr_map and subsequently returns the
    /// AccountAddress associated to the PrivateKey, along with it's ChildNumber
    /// 生成新密钥并将其添加到addr_map并随后返回与PrivateKey关联的AccountAddress及其子编号的函数
    pub fn new_address(&mut self) -> Result<(AccountAddress, ChildNumber)> {
        let child = self.key_factory.private_child(self.key_leaf)?;
        let address = child.get_address()?;
        let child = self.key_leaf;
        self.key_leaf.increment();
        match self.addr_map.insert(address, child) {
            Some(_) => Err(WalletError::LibraWalletGeneric(
                "This address is already in your wallet".to_string(),
            )),
            None => Ok((address, child)),
        }
    }

    /// Returns a list of all addresses controlled by this wallet that are currently held by the
    /// addr_map
    /// 返回此钱包控制的当前由addr_map持有的所有地址的列表
    pub fn get_addresses(&self) -> Result<Vec<AccountAddress>> {
        let mut ret = Vec::with_capacity(self.addr_map.len());
        let rev_map = self
            .addr_map
            .iter()
            .map(|(&k, &v)| (v.as_ref().to_owned(), k.to_owned()))
            .collect::<HashMap<_, _>>();
        for i in 0..self.addr_map.len() as u64 {
            match rev_map.get(&i) {
                Some(account_address) => {
                    ret.push(*account_address);
                }
                None => {
                    return Err(WalletError::LibraWalletGeneric(format!(
                        "Child num {} not exist while depth is {}",
                        i,
                        self.addr_map.len()
                    )))
                }
            }
        }
        Ok(ret)
    }

    /// Simple public function that allows to sign a Libra RawTransaction with the PrivateKey
    /// associated to a particular AccountAddress. If the PrivateKey associated to an
    /// AccountAddress is not contained in the addr_map, then this function will return an Error
    /// 简单的公共功能，允许使用与特定AccountAddress关联的PrivateKey签名Libra RawTransaction。 如果与
    /// AccountAddress关联的私钥未包含在addr_map中，则此函数将返回错误
    pub fn sign_txn(&self, txn: RawTransaction) -> Result<SignedTransaction> {
        if let Some(child) = self.addr_map.get(&txn.sender()) {
            let raw_bytes = txn.into_proto().write_to_bytes()?;
            let txn_hashvalue = RawTransactionBytes(&raw_bytes).hash();

            let child_key = self.key_factory.private_child(child.clone())?;
            let signature = child_key.sign(txn_hashvalue);
            let public_key = child_key.get_public();

            let mut signed_txn = ProtoSignedTransaction::new();
            signed_txn.set_raw_txn_bytes(raw_bytes.to_vec());
            signed_txn.set_sender_public_key(public_key.to_bytes().to_vec());
            signed_txn.set_sender_signature(signature.to_bytes().to_vec());

            Ok(SignedTransaction::from_proto(signed_txn)?)
        } else {
            Err(WalletError::LibraWalletGeneric(
                "Well, that address is nowhere to be found... This is awkward".to_string(),
            ))
        }
    }
}

/// WalletLibrary naturally support TransactionSigner trait.
/// WalletLibrary自然支持TransactionSigner特性。
impl TransactionSigner for WalletLibrary {
    fn sign_txn(&self, raw_txn: RawTransaction) -> failure::prelude::Result<SignedTransaction> {
        Ok(self.sign_txn(raw_txn)?)
    }
}
