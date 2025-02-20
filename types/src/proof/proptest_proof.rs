// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! All proofs generated in this module are not valid proofs. They are only for the purpose of
//! testing conversion between Rust and Protobuf.
//! 本模块中生成的所有证明都不是有效证明。 它们仅用于此目的测试Rust和Protobuf之间的转换。

use crate::{
    proof::{
        AccountStateProof, AccumulatorProof, EventProof, SignedTransactionProof, SparseMerkleProof,
    },
    transaction::TransactionInfo,
};
use crypto::{
    hash::{ACCUMULATOR_PLACEHOLDER_HASH, SPARSE_MERKLE_PLACEHOLDER_HASH},
    HashValue,
};
use proptest::{collection::vec, prelude::*};
use rand::{seq::SliceRandom, thread_rng};

prop_compose! {
    fn arb_accumulator_proof()(
        non_default_siblings in vec(any::<HashValue>(), 0..63usize),
        total_num_siblings in 0..64usize,
    ) -> AccumulatorProof {
        let mut siblings = non_default_siblings;
        if !siblings.is_empty() {
            let total_num_siblings = std::cmp::max(siblings.len(), total_num_siblings);
            for _ in siblings.len()..total_num_siblings {
                siblings.push(ACCUMULATOR_PLACEHOLDER_HASH.clone());
            }
            assert_eq!(siblings.len(), total_num_siblings);
            (&mut siblings[1..]).shuffle(&mut thread_rng());
        }
        AccumulatorProof::new(siblings)
    }
}

prop_compose! {
    fn arb_sparse_merkle_proof()(
        leaf in any::<Option<(HashValue, HashValue)>>(),
        non_default_siblings in vec(any::<HashValue>(), 0..256usize),
        total_num_siblings in 0..257usize,
    ) -> SparseMerkleProof {
        let mut siblings = non_default_siblings;
        if !siblings.is_empty() {
            let total_num_siblings = std::cmp::max(siblings.len(), total_num_siblings);
            for _ in siblings.len()..total_num_siblings {
                siblings.insert(0, SPARSE_MERKLE_PLACEHOLDER_HASH.clone());
            }
            assert_eq!(siblings.len(), total_num_siblings);
            (&mut siblings[0..total_num_siblings-1]).shuffle(&mut thread_rng());
        }
        SparseMerkleProof::new(leaf, siblings)
    }
}

prop_compose! {
    fn arb_signed_transaction_proof()(
        ledger_info_to_transaction_info_proof in any::<AccumulatorProof>(),
        transaction_info in any::<TransactionInfo>(),
    ) -> SignedTransactionProof {
        SignedTransactionProof::new(ledger_info_to_transaction_info_proof, transaction_info)
    }
}

prop_compose! {
    fn arb_account_state_proof()(
        ledger_info_to_transaction_info_proof in any::<AccumulatorProof>(),
        transaction_info in any::<TransactionInfo>(),
        transaction_info_to_account_proof in any::<SparseMerkleProof>(),
    ) -> AccountStateProof {
        AccountStateProof::new(
            ledger_info_to_transaction_info_proof,
            transaction_info,
            transaction_info_to_account_proof,
        )
    }
}

prop_compose! {
    fn arb_event_proof()(
        ledger_info_to_transaction_info_proof in any::<AccumulatorProof>(),
        transaction_info in any::<TransactionInfo>(),
        transaction_info_to_event_proof in any::<AccumulatorProof>(),
    ) -> EventProof {
        EventProof::new(
            ledger_info_to_transaction_info_proof,
            transaction_info,
            transaction_info_to_event_proof,
        )
    }
}

macro_rules! impl_arbitrary_for_proof {
    ($proof_type: ident, $arb_func: ident) => {
        impl Arbitrary for $proof_type {
            type Parameters = ();
            type Strategy = BoxedStrategy<Self>;

            fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
                $arb_func().boxed()
            }
        }
    };
}

impl_arbitrary_for_proof!(AccumulatorProof, arb_accumulator_proof);
impl_arbitrary_for_proof!(SparseMerkleProof, arb_sparse_merkle_proof);
impl_arbitrary_for_proof!(SignedTransactionProof, arb_signed_transaction_proof);
impl_arbitrary_for_proof!(AccountStateProof, arb_account_state_proof);
impl_arbitrary_for_proof!(EventProof, arb_event_proof);
