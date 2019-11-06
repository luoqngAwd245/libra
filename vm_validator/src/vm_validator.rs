// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use config::config::NodeConfig;
use failure::prelude::*;
use futures::future::{err, ok, Future};
use scratchpad::SparseMerkleTree;
use std::sync::Arc;
use storage_client::{StorageRead, VerifiedStateView};
use types::{
    account_address::{AccountAddress, ADDRESS_LENGTH},
    account_config::get_account_resource_or_default,
    get_with_proof::{RequestItem, ResponseItem},
    transaction::SignedTransaction,
    vm_error::VMStatus,
};
use vm_runtime::{MoveVM, VMVerifier};

#[cfg(test)]
#[path = "unit_tests/vm_validator_test.rs"]
mod vm_validator_test;

pub trait TransactionValidation: Send + Sync {
    type ValidationInstance: VMVerifier;
    /// Validate a txn from client
    /// 从客户端验证交易
    fn validate_transaction(
        &self,
        _txn: SignedTransaction,
    ) -> Box<dyn Future<Item = Option<VMStatus>, Error = failure::Error> + Send>;
}

#[derive(Clone)]
pub struct VMValidator {
    storage_read_client: Arc<dyn StorageRead>,
    vm: MoveVM,
}

impl VMValidator {
    pub fn new(config: &NodeConfig, storage_read_client: Arc<dyn StorageRead>) -> Self {
        VMValidator {
            storage_read_client,
            vm: MoveVM::new(&config.vm_config),
        }
    }
}

impl TransactionValidation for VMValidator {
    type ValidationInstance = MoveVM;

    fn validate_transaction(
        &self,
        txn: SignedTransaction,
    ) -> Box<dyn Future<Item = Option<VMStatus>, Error = failure::Error> + Send> {
        // TODO: For transaction validation, there are two options to go:
        // 1. Trust storage: there is no need to get root hash from storage here. We will
        // create another struct similar to `VerifiedStateView` that implements `StateView`
        // but does not do verification.
        // 2. Don't trust storage. This requires more work:
        // 1) AC must have validator set information
        // 2) Get state_root from transaction info which can be verified with signatures of
        // validator set.
        // 3) Create VerifiedStateView with verified state
        // root.
        // 对于交易验证，这里有两种选择
        // 1.信任的存储：没有必要从存储获取根hash。我们将创建另外一个和`VerifiedStateView`相似的结构
        // 这个结构实现了`StateView`但是不做验证
        // 2.不信任的存储。需要更多的工作：
        // 1) AC 必须有验证节点的组信息
        // 2）从事务信息中获取state_root，可以使用验证器集的签名进行验证。
        // 3）使用验证状态创建VerifiedStateView


        // Just ask something from storage. It doesn't matter what it is -- we just need the
        // transaction info object in account state proof which contains the state root hash.
        // 从存储中询问一些东西。它是什么并不重要 - 我们只需要在帐户状态证明中包含状态根哈希的事务信息对象。
        let address = AccountAddress::new([0xff; ADDRESS_LENGTH]);
        let item = RequestItem::GetAccountState { address };

        match self
            .storage_read_client
            .update_to_latest_ledger(/* client_known_version = */ 0, vec![item])
        {
            Ok((mut items, _, _)) => {
                if items.len() != 1 {
                    return Box::new(err(format_err!(
                        "Unexpected number of items ({}).",
                        items.len()
                    )
                    .into()));
                }

                match items.remove(0) {
                    ResponseItem::GetAccountState {
                        account_state_with_proof,
                    } => {
                        let transaction_info = account_state_with_proof.proof.transaction_info();
                        let state_root = transaction_info.state_root_hash();
                        // 稀疏默克尔树
                        let smt = SparseMerkleTree::new(state_root);
                        let state_view = VerifiedStateView::new(
                            Arc::clone(&self.storage_read_client),
                            state_root,
                            &smt,
                        );
                        Box::new(ok(self.vm.validate_transaction(txn, &state_view)))
                    }
                    _ => panic!("Unexpected item in response."),
                }
            }
            Err(e) => Box::new(err(e.into())),
        }
    }
}

/// read account state
/// returns account's current sequence number and balance
/// 读取账户状态
/// 返回状态当前序列号和余额
pub async fn get_account_state(
    storage_read_client: Arc<dyn StorageRead>,
    address: AccountAddress,
) -> Result<(u64, u64)> {
    let req_item = RequestItem::GetAccountState { address };
    let (response_items, _, _) = storage_read_client
        .update_to_latest_ledger_async(0 /* client_known_version */, vec![req_item])
        .await?;
    let account_state = match &response_items[0] {
        ResponseItem::GetAccountState {
            account_state_with_proof,
        } => &account_state_with_proof.blob,
        _ => bail!("Not account state response."),
    };
    let account_resource = get_account_resource_or_default(account_state)?;
    let sequence_number = account_resource.sequence_number();
    let balance = account_resource.balance();
    Ok((sequence_number, balance))
}
