// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! This module has definition of various proofs.
//! 该模块定义了各种证明。

#[cfg(test)]
#[path = "unit_tests/proof_proto_conversion_test.rs"]
mod proof_proto_conversion_test;

use self::bitmap::{AccumulatorBitmap, SparseMerkleBitmap};
use crate::transaction::TransactionInfo;
use crypto::{
    hash::{ACCUMULATOR_PLACEHOLDER_HASH, SPARSE_MERKLE_PLACEHOLDER_HASH},
    HashValue,
};
use failure::prelude::*;
use proto_conv::{FromProto, IntoProto};

/// A proof that can be used authenticate an element in an accumulator given trusted root hash. For
/// example, both `LedgerInfoToTransactionInfoProof` and `TransactionInfoToEventProof` can be
/// constructed on top of this structure.
/// 可以使用的证明在给定受信任的根哈希值的情况下验证累加器中的元素。 例如，可以在此结构之上构造
/// `LedgerInfoToTransactionInfoProof`和`TransactionInfoToEventProof`。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccumulatorProof {
    /// All siblings in this proof, including the default ones. Siblings near the root are at the
    /// beginning of the vector.
    /// 此证明中的所有兄弟姐妹，包括默认的兄弟姐妹。 根附近的兄弟姐妹位于向量的开头。
    siblings: Vec<HashValue>,
}

impl AccumulatorProof {
    /// Constructs a new `AccumulatorProof` using a list of siblings.
    /// 使用兄弟姐妹列表构造一个新的“AccumulatorProof”。
    pub fn new(siblings: Vec<HashValue>) -> Self {
        // The sibling list could be empty in case the accumulator is empty or has a single
        // element. When it's not empty, the top most sibling will never be default, otherwise the
        // accumulator should have collapsed to a smaller one.
        // 如果累加器为空或具有单个元素，则兄弟列表可以为空。 当它不为空时，最顶层的兄弟将永远不会默认，
        // 否则累加器应该折叠成较小的。
        if let Some(first_sibling) = siblings.first() {
            assert_ne!(*first_sibling, *ACCUMULATOR_PLACEHOLDER_HASH);
        }

        AccumulatorProof { siblings }
    }

    /// Returns the list of siblings in this proof.
    /// 返回此证明中的兄弟列表。
    pub fn siblings(&self) -> &[HashValue] {
        &self.siblings
    }
}

impl FromProto for AccumulatorProof {
    type ProtoType = crate::proto::proof::AccumulatorProof;

    fn from_proto(mut proto_proof: Self::ProtoType) -> Result<Self> {
        let bitmap = proto_proof.get_bitmap();
        let num_non_default_siblings = bitmap.count_ones() as usize;
        ensure!(
            num_non_default_siblings == proto_proof.get_non_default_siblings().len(),
            "Malformed proof. Bitmap indicated {} non-default siblings. Found {} siblings.",
            num_non_default_siblings,
            proto_proof.get_non_default_siblings().len()
        );

        let mut proto_siblings = proto_proof.take_non_default_siblings().into_iter();
        // Iterate from the leftmost 1-bit to LSB in the bitmap. If a bit is set, the corresponding
        // sibling is non-default and we take the sibling from proto_siblings.  Otherwise the
        // sibling on this position is default.
        // 从位图中最左边的1位到LSB迭代。 如果设置了一个位，则相应的兄弟是非默认的，我们从proto_siblings
        // 中获取兄弟。 否则，此位置的兄弟是默认的。
        let siblings = AccumulatorBitmap::new(bitmap)
            .iter()
            .map(|x| {
                if x {
                    let hash_bytes = proto_siblings
                        .next()
                        .expect("Unexpected number of siblings.");
                    HashValue::from_slice(&hash_bytes)
                } else {
                    Ok(*ACCUMULATOR_PLACEHOLDER_HASH)
                }
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(AccumulatorProof::new(siblings))
    }
}

impl IntoProto for AccumulatorProof {
    type ProtoType = crate::proto::proof::AccumulatorProof;

    fn into_proto(self) -> Self::ProtoType {
        let mut proto_proof = Self::ProtoType::new();
        // Iterate over all siblings. For each non-default sibling, add to protobuf struct and set
        // the corresponding bit in the bitmap.
        // 迭代所有兄弟姐妹。 对于每个非默认同级，添加到protobuf结构并在位图中设置相应的位。
        let bitmap: AccumulatorBitmap = self
            .siblings
            .into_iter()
            .map(|sibling| {
                if sibling != *ACCUMULATOR_PLACEHOLDER_HASH {
                    proto_proof
                        .mut_non_default_siblings()
                        .push(sibling.to_vec());
                    true
                } else {
                    false
                }
            })
            .collect();
        proto_proof.set_bitmap(bitmap.into());
        proto_proof
    }
}

/// A proof that can be used to authenticate an element in a Sparse Merkle Tree given trusted root
/// hash. For example, `TransactionInfoToAccountProof` can be constructed on top of this structure.
/// 可用于在给定受信任根哈希的情况下验证稀疏Merkle树中的元素的证明。 例如，可以在此结构之上构造
/// `TransactionInfoToAccountProof`。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SparseMerkleProof {
    /// This proof can be used to authenticate whether a given leaf exists in the tree or not.
    ///     - If this is `Some(HashValue, HashValue)`
    ///         - If the first `HashValue` equals requested key, this is an inclusion proof and the
    ///           second `HashValue` equals the hash of the corresponding account blob.
    ///         - Otherwise this is a non-inclusion proof. The first `HashValue` is the only key
    ///           that exists in the subtree and the second `HashValue` equals the hash of the
    ///           corresponding account blob.
    ///     - If this is `None`, this is also a non-inclusion proof which indicates the subtree is
    ///       empty.
    /// 此证明可用于验证树中是否存在给定叶。
    /// - 如果这是`Some（HashValue，HashValue）`
    /// - 如果第一个“HashValue”等于请求的密钥，则这是包含证明，第二个“HashValue”等于相应帐户blob的散列。
    /// - 否则这是非包含证明。 第一个`HashValue`是子树中唯一存在的键，第二个`HashValue`等于相应帐号blob的哈希值。
    /// - 如果这是“无”，这也是一个非包含证明，表明子树是空的。
    leaf: Option<(HashValue, HashValue)>,

    /// All siblings in this proof, including the default ones. Siblings near the root are at the
    /// beginning of the vector.
    /// 此证明中的所有兄弟姐妹，包括默认的兄弟姐妹。 根附近的兄弟姐妹位于向量的开头。
    siblings: Vec<HashValue>,
}

impl SparseMerkleProof {
    /// Constructs a new `SparseMerkleProof` using leaf and a list of siblings.
    /// 使用leaf和兄弟列表构造一个新的`SparseMerkleProof`。
    pub fn new(leaf: Option<(HashValue, HashValue)>, siblings: Vec<HashValue>) -> Self {
        // The sibling list could be empty in case the Sparse Merkle Tree is empty or has a single
        // element. When it's not empty, the bottom most sibling will never be default, otherwise a
        // leaf and a default sibling should have collapsed to a leaf.
        // 如果稀疏Merkle树为空或具有单个元素，则兄弟列表可以为空。 如果它不是空的，那么最底层的兄弟
        // 将永远不会是默认的，否则叶子和默认的兄弟应该折叠成叶子。
        if let Some(last_sibling) = siblings.last() {
            assert_ne!(*last_sibling, *SPARSE_MERKLE_PLACEHOLDER_HASH);
        }

        SparseMerkleProof { leaf, siblings }
    }

    /// Returns the leaf node in this proof.
    /// 返回此证明中的叶节点。
    pub fn leaf(&self) -> Option<(HashValue, HashValue)> {
        self.leaf
    }

    /// Returns the list of siblings in this proof.
    /// 返回此证明中的兄弟列表。
    pub fn siblings(&self) -> &[HashValue] {
        &self.siblings
    }
}

impl FromProto for SparseMerkleProof {
    type ProtoType = crate::proto::proof::SparseMerkleProof;

    /// Validates `proto_proof` and converts it to `Self` if validation passed.
    /// 如果验证通过，验证`proto_proof`并将其转换为`Self`。
    fn from_proto(mut proto_proof: Self::ProtoType) -> Result<Self> {
        let proto_leaf = proto_proof.take_leaf();
        let leaf = if proto_leaf.is_empty() {
            None
        } else if proto_leaf.len() == HashValue::LENGTH * 2 {
            let key = HashValue::from_slice(&proto_leaf[0..HashValue::LENGTH])?;
            let value_hash = HashValue::from_slice(&proto_leaf[HashValue::LENGTH..])?;
            Some((key, value_hash))
        } else {
            bail!(
                "Mailformed proof. Leaf has {} bytes. Expect 0 or {} bytes.",
                proto_leaf.len(),
                HashValue::LENGTH * 2
            );
        };

        let bitmap = proto_proof.take_bitmap();
        if let Some(last_byte) = bitmap.last() {
            ensure!(
                *last_byte != 0,
                "Malformed proof. The last byte of the bitmap is zero."
            );
        }
        let num_non_default_siblings = bitmap.iter().fold(0, |total, x| total + x.count_ones());
        ensure!(
            num_non_default_siblings as usize == proto_proof.get_non_default_siblings().len(),
            "Malformed proof. Bitmap indicated {} non-default siblings. Found {} siblings.",
            num_non_default_siblings,
            proto_proof.get_non_default_siblings().len()
        );

        let mut proto_siblings = proto_proof.take_non_default_siblings().into_iter();
        // Iterate from the MSB of the first byte to the rightmost 1-bit in the bitmap. If a bit is
        // set, the corresponding sibling is non-default and we take the sibling from
        // proto_siblings. Otherwise the sibling on this position is default.
        // 从位图的第一个字节的MSB到最右边的1位迭代。 如果设置了一个位，则相应的兄弟是非默认的，我们从
        // proto_siblings中获取兄弟。 否则，此位置的兄弟是默认的。
        let siblings: Result<Vec<_>> = SparseMerkleBitmap::new(bitmap)
            .iter()
            .map(|x| {
                if x {
                    let hash_bytes = proto_siblings
                        .next()
                        .expect("Unexpected number of siblings.");
                    HashValue::from_slice(&hash_bytes)
                } else {
                    Ok(*SPARSE_MERKLE_PLACEHOLDER_HASH)
                }
            })
            .collect();

        Ok(SparseMerkleProof::new(leaf, siblings?))
    }
}

impl IntoProto for SparseMerkleProof {
    type ProtoType = crate::proto::proof::SparseMerkleProof;

    fn into_proto(self) -> Self::ProtoType {
        let mut proto_proof = Self::ProtoType::new();
        // If a leaf is present, we write the key and value hash as a single byte array of 64
        // bytes. Otherwise we write an empty byte array.
        // 如果存在叶子，我们将键和值散列写为64字节的单字节数组。 否则我们写一个空字节数组。
        if let Some((key, value_hash)) = self.leaf {
            proto_proof.mut_leaf().extend_from_slice(key.as_ref());
            proto_proof
                .mut_leaf()
                .extend_from_slice(value_hash.as_ref());
        }
        // Iterate over all siblings. For each non-default sibling, add to protobuf struct and set
        // the corresponding bit in the bitmap.
        // 迭代所有兄弟姐妹。 对于每个非默认同级，添加到protobuf结构并在位图中设置相应的位。
        let bitmap: SparseMerkleBitmap = self
            .siblings
            .into_iter()
            .map(|sibling| {
                if sibling != *SPARSE_MERKLE_PLACEHOLDER_HASH {
                    proto_proof
                        .mut_non_default_siblings()
                        .push(sibling.to_vec());
                    true
                } else {
                    false
                }
            })
            .collect();
        proto_proof.set_bitmap(bitmap.into());
        proto_proof
    }
}

/// The complete proof used to authenticate a `SignedTransaction` object.  This structure consists
/// of an `AccumulatorProof` from `LedgerInfo` to `TransactionInfo` the verifier needs to verify
/// the correctness of the `TransactionInfo` object, and the `TransactionInfo` object that is
/// supposed to match the `SignedTransaction`.
/// 用于验证`SignedTransaction`对象的完整证明。 这个结构包含一个``AccumulatorProof`，从`LedgerInfo`到
/// `TransactionInfo`，验证者需要验证`TransactionInfo`对象的正确性，以及应该与`SignedTransaction`匹配的`TransactionInfo`对象。
#[derive(Clone, Debug, Eq, PartialEq, FromProto, IntoProto)]
#[ProtoType(crate::proto::proof::SignedTransactionProof)]
pub struct SignedTransactionProof {
    /// The accumulator proof from ledger info root to leaf that authenticates the hash of the
    /// `TransactionInfo` object.
    /// 累加器从分类帐info root到leaf验证`TransactionInfo`对象的散列。
    ledger_info_to_transaction_info_proof: AccumulatorProof,

    /// The `TransactionInfo` object at the leaf of the accumulator.
    /// 累加器叶子上的`TransactionInfo`对象。
    transaction_info: TransactionInfo,
}

impl SignedTransactionProof {
    /// Constructs a new `SignedTransactionProof` object using given
    /// `ledger_info_to_transaction_info_proof`.
    /// 使用给定的`ledger_info_to_transaction_info_proof`构造一个新的`SignedTransactionProof`对象。
    pub fn new(
        ledger_info_to_transaction_info_proof: AccumulatorProof,
        transaction_info: TransactionInfo,
    ) -> Self {
        SignedTransactionProof {
            ledger_info_to_transaction_info_proof,
            transaction_info,
        }
    }

    /// Returns the `ledger_info_to_transaction_info_proof` object in this proof.
    /// 返回此证明中的`ledger_info_to_transaction_info_proof`对象。
    pub fn ledger_info_to_transaction_info_proof(&self) -> &AccumulatorProof {
        &self.ledger_info_to_transaction_info_proof
    }

    /// Returns the `transaction_info` object in this proof.
    /// 返回此证明中的`transaction_info`对象。
    pub fn transaction_info(&self) -> &TransactionInfo {
        &self.transaction_info
    }
}

/// The complete proof used to authenticate the state of an account. This structure consists of the
/// `AccumulatorProof` from `LedgerInfo` to `TransactionInfo`, the `TransactionInfo` object and the
/// `SparseMerkleProof` from state root to the account.
/// 用于验证帐户状态的完整证据。 这个结构包括从`LedgerInfo`到`TransactionInfo`的`AccumulatorProof`，
/// `TransactionInfo`对象和从州根到帐户的`SparseMerkleProof`。
#[derive(Clone, Debug, Eq, PartialEq, FromProto, IntoProto)]
#[ProtoType(crate::proto::proof::AccountStateProof)]
pub struct AccountStateProof {
    /// The accumulator proof from ledger info root to leaf that authenticates the hash of the
    /// `TransactionInfo` object.
    /// 累加器从分类帐info root到leaf验证`TransactionInfo`对象的散列。
    ledger_info_to_transaction_info_proof: AccumulatorProof,

    /// The `TransactionInfo` object at the leaf of the accumulator.
    /// 累加器叶子上的`TransactionInfo`对象。
    transaction_info: TransactionInfo,

    /// The sparse merkle proof from state root to the account state.
    /// 从状态根到帐户状态的稀疏merkle证明。
    transaction_info_to_account_proof: SparseMerkleProof,
}

impl AccountStateProof {
    /// Constructs a new `AccountStateProof` using given `ledger_info_to_transaction_info_proof`,
    /// `transaction_info` and `transaction_info_to_account_proof`.
    /// 使用给定的`ledger_info_to_transaction_info_proof`，`transaction_info`和
    /// `transaction_info_to_account_proof`构造一个新的`AccountStateProof`。
    pub fn new(
        ledger_info_to_transaction_info_proof: AccumulatorProof,
        transaction_info: TransactionInfo,
        transaction_info_to_account_proof: SparseMerkleProof,
    ) -> Self {
        AccountStateProof {
            ledger_info_to_transaction_info_proof,
            transaction_info,
            transaction_info_to_account_proof,
        }
    }

    /// Returns the `ledger_info_to_transaction_info_proof` object in this proof.
    /// 返回此证明中的`ledger_info_to_transaction_info_proof`对象。
    pub fn ledger_info_to_transaction_info_proof(&self) -> &AccumulatorProof {
        &self.ledger_info_to_transaction_info_proof
    }

    /// Returns the `transaction_info` object in this proof.
    /// 返回此证明中的`transaction_info`对象。
    pub fn transaction_info(&self) -> &TransactionInfo {
        &self.transaction_info
    }

    /// Returns the `transaction_info_to_account_proof` object in this proof.
    /// 返回此证明中的`transaction_info_to_account_proof`对象。
    pub fn transaction_info_to_account_proof(&self) -> &SparseMerkleProof {
        &self.transaction_info_to_account_proof
    }
}

/// The complete proof used to authenticate a contract event. This structure consists of the
/// `AccumulatorProof` from `LedgerInfo` to `TransactionInfo`, the `TransactionInfo` object and the
/// `AccumulatorProof` from event accumulator root to the event.
/// 用于验证合同事件的完整证据。 这个结构包括从`LedgerInfo`到`TransactionInfo`的`AccumulatorProof`，
/// `TransactionInfo`对象和从事件累加器根到事件的`AccumulatorProof`。
#[derive(Clone, Debug, Eq, PartialEq, FromProto, IntoProto)]
#[ProtoType(crate::proto::proof::EventProof)]
pub struct EventProof {
    /// The accumulator proof from ledger info root to leaf that authenticates the hash of the
    /// `TransactionInfo` object.
    /// 累加器从分类帐info root到leaf验证`TransactionInfo`对象的散列。
    ledger_info_to_transaction_info_proof: AccumulatorProof,

    /// The `TransactionInfo` object at the leaf of the accumulator.
    /// 累加器叶子上的`TransactionInfo`对象。
    transaction_info: TransactionInfo,

    /// The accumulator proof from event root to the actual event.
    /// 累加器从事件根证明到实际事件。
    transaction_info_to_event_proof: AccumulatorProof,
}

impl EventProof {
    /// Constructs a new `EventProof` using given `ledger_info_to_transaction_info_proof`,
    /// `transaction_info` and `transaction_info_to_event_proof`.
    /// 使用给定的`ledger_info_to_transaction_info_proof`构造一个新的`EventProof`，
    ///  `transaction_info`和`transaction_info_to_event_proof`。
    pub fn new(
        ledger_info_to_transaction_info_proof: AccumulatorProof,
        transaction_info: TransactionInfo,
        transaction_info_to_event_proof: AccumulatorProof,
    ) -> Self {
        EventProof {
            ledger_info_to_transaction_info_proof,
            transaction_info,
            transaction_info_to_event_proof,
        }
    }

    /// Returns the `ledger_info_to_transaction_info_proof` object in this proof.
    pub fn ledger_info_to_transaction_info_proof(&self) -> &AccumulatorProof {
        &self.ledger_info_to_transaction_info_proof
    }

    /// Returns the `transaction_info` object in this proof.
    pub fn transaction_info(&self) -> &TransactionInfo {
        &self.transaction_info
    }

    /// Returns the `transaction_info_to_event_proof` object in this proof.
    pub fn transaction_info_to_event_proof(&self) -> &AccumulatorProof {
        &self.transaction_info_to_event_proof
    }
}

mod bitmap {
    /// The bitmap indicating which siblings are default in a compressed accumulator proof. 1 means
    /// non-default and 0 means default.  The LSB corresponds to the sibling at the bottom of the
    /// accumulator. The leftmost 1-bit corresponds to the sibling at the top of the accumulator,
    /// since this one is always non-default.
    /// 位图指示哪个兄弟节点在压缩累加器证明中是默认的。 1表示非默认值，0表示默认值。
    /// LSB对应于累加器底部的兄弟。 最左边的1位对应于累加器顶部的兄弟，因为这一个总是非默认的。
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct AccumulatorBitmap(u64);

    impl AccumulatorBitmap {
        pub fn new(bitmap: u64) -> Self {
            AccumulatorBitmap(bitmap)
        }

        pub fn iter(self) -> AccumulatorBitmapIterator {
            AccumulatorBitmapIterator::new(self.0)
        }
    }

    impl std::convert::From<AccumulatorBitmap> for u64 {
        fn from(bitmap: AccumulatorBitmap) -> u64 {
            bitmap.0
        }
    }

    /// Given a u64 bitmap, this iterator generates one bit at a time starting from the leftmost
    /// 1-bit.
    pub struct AccumulatorBitmapIterator {
        bitmap: AccumulatorBitmap,
        mask: u64,
    }

    impl AccumulatorBitmapIterator {
        fn new(bitmap: u64) -> Self {
            let num_leading_zeros = bitmap.leading_zeros();
            let mask = if num_leading_zeros >= 64 {
                0
            } else {
                1 << (63 - num_leading_zeros)
            };
            AccumulatorBitmapIterator {
                bitmap: AccumulatorBitmap(bitmap),
                mask,
            }
        }
    }

    impl std::iter::Iterator for AccumulatorBitmapIterator {
        type Item = bool;

        fn next(&mut self) -> Option<bool> {
            if self.mask == 0 {
                return None;
            }
            let ret = self.bitmap.0 & self.mask != 0;
            self.mask >>= 1;
            Some(ret)
        }
    }

    impl std::iter::FromIterator<bool> for AccumulatorBitmap {
        fn from_iter<I>(iter: I) -> Self
        where
            I: std::iter::IntoIterator<Item = bool>,
        {
            let mut bitmap = 0;
            for (i, bit) in iter.into_iter().enumerate() {
                if i == 0 {
                    assert!(bit, "The first bit should always be set.");
                } else if i > 63 {
                    panic!("Trying to put more than 64 bits in AccumulatorBitmap.");
                }
                bitmap <<= 1;
                bitmap |= bit as u64;
            }
            AccumulatorBitmap::new(bitmap)
        }
    }

    /// The bitmap indicating which siblings are default in a compressed sparse merkle proof. 1
    /// means non-default and 0 means default.  The MSB of the first byte corresponds to the
    /// sibling at the top of the Sparse Merkle Tree. The rightmost 1-bit of the last byte
    /// corresponds to the sibling at the bottom, since this one is always non-default.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct SparseMerkleBitmap(Vec<u8>);

    impl SparseMerkleBitmap {
        pub fn new(bitmap: Vec<u8>) -> Self {
            SparseMerkleBitmap(bitmap)
        }

        pub fn iter(&self) -> SparseMerkleBitmapIterator {
            SparseMerkleBitmapIterator::new(&self.0)
        }
    }

    impl std::convert::From<SparseMerkleBitmap> for Vec<u8> {
        fn from(bitmap: SparseMerkleBitmap) -> Vec<u8> {
            bitmap.0
        }
    }

    /// Given a `Vec<u8>` bitmap, this iterator generates one bit at a time starting from the MSB
    /// of the first byte. All trailing zeros of the last byte are discarded.
    pub struct SparseMerkleBitmapIterator<'a> {
        bitmap: &'a [u8],
        index: usize,
        len: usize,
    }

    impl<'a> SparseMerkleBitmapIterator<'a> {
        fn new(bitmap: &'a [u8]) -> Self {
            match bitmap.last() {
                Some(last_byte) => {
                    assert_ne!(
                        *last_byte, 0,
                        "The last byte of the bitmap should never be zero."
                    );
                    SparseMerkleBitmapIterator {
                        bitmap,
                        index: 0,
                        len: bitmap.len() * 8 - last_byte.trailing_zeros() as usize,
                    }
                }
                None => SparseMerkleBitmapIterator {
                    bitmap,
                    index: 0,
                    len: 0,
                },
            }
        }
    }

    impl<'a> std::iter::Iterator for SparseMerkleBitmapIterator<'a> {
        type Item = bool;

        fn next(&mut self) -> Option<bool> {
            // We are past the last useful bit.
            if self.index >= self.len {
                return None;
            }

            let pos = self.index / 8;
            let bit = self.index % 8;
            let ret = self.bitmap[pos] >> (7 - bit) & 1 != 0;
            self.index += 1;
            Some(ret)
        }
    }

    impl std::iter::FromIterator<bool> for SparseMerkleBitmap {
        fn from_iter<I>(iter: I) -> Self
        where
            I: std::iter::IntoIterator<Item = bool>,
        {
            let mut bitmap = vec![];
            for (i, bit) in iter.into_iter().enumerate() {
                let pos = i % 8;
                if pos == 0 {
                    bitmap.push(0);
                }
                let last_byte = bitmap
                    .last_mut()
                    .expect("The bitmap vector should not be empty");
                *last_byte |= (bit as u8) << (7 - pos);
            }
            SparseMerkleBitmap::new(bitmap)
        }
    }
}
