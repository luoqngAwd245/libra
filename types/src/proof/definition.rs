// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! This module has definition of various proofs.
//! ��ģ�鶨���˸���֤����

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
/// ����ʹ�õ�֤���ڸ��������εĸ���ϣֵ���������֤�ۼ����е�Ԫ�ء� ���磬�����ڴ˽ṹ֮�Ϲ���
/// `LedgerInfoToTransactionInfoProof`��`TransactionInfoToEventProof`��
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccumulatorProof {
    /// All siblings in this proof, including the default ones. Siblings near the root are at the
    /// beginning of the vector.
    /// ��֤���е������ֵܽ��ã�����Ĭ�ϵ��ֵܽ��á� ���������ֵܽ���λ�������Ŀ�ͷ��
    siblings: Vec<HashValue>,
}

impl AccumulatorProof {
    /// Constructs a new `AccumulatorProof` using a list of siblings.
    /// ʹ���ֵܽ����б���һ���µġ�AccumulatorProof����
    pub fn new(siblings: Vec<HashValue>) -> Self {
        // The sibling list could be empty in case the accumulator is empty or has a single
        // element. When it's not empty, the top most sibling will never be default, otherwise the
        // accumulator should have collapsed to a smaller one.
        // ����ۼ���Ϊ�ջ���е���Ԫ�أ����ֵ��б����Ϊ�ա� ������Ϊ��ʱ�������ֵܽ���Զ����Ĭ�ϣ�
        // �����ۼ���Ӧ���۵��ɽ�С�ġ�
        if let Some(first_sibling) = siblings.first() {
            assert_ne!(*first_sibling, *ACCUMULATOR_PLACEHOLDER_HASH);
        }

        AccumulatorProof { siblings }
    }

    /// Returns the list of siblings in this proof.
    /// ���ش�֤���е��ֵ��б�
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
        // ��λͼ������ߵ�1λ��LSB������ ���������һ��λ������Ӧ���ֵ��Ƿ�Ĭ�ϵģ����Ǵ�proto_siblings
        // �л�ȡ�ֵܡ� ���򣬴�λ�õ��ֵ���Ĭ�ϵġ�
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
        // ���������ֵܽ��á� ����ÿ����Ĭ��ͬ������ӵ�protobuf�ṹ����λͼ��������Ӧ��λ��
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
/// �������ڸ��������θ���ϣ���������֤ϡ��Merkle���е�Ԫ�ص�֤���� ���磬�����ڴ˽ṹ֮�Ϲ���
/// `TransactionInfoToAccountProof`��
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
    /// ��֤����������֤�����Ƿ���ڸ���Ҷ��
    /// - �������`Some��HashValue��HashValue��`
    /// - �����һ����HashValue�������������Կ�������ǰ���֤�����ڶ�����HashValue��������Ӧ�ʻ�blob��ɢ�С�
    /// - �������Ƿǰ���֤���� ��һ��`HashValue`��������Ψһ���ڵļ����ڶ���`HashValue`������Ӧ�ʺ�blob�Ĺ�ϣֵ��
    /// - ������ǡ��ޡ�����Ҳ��һ���ǰ���֤�������������ǿյġ�
    leaf: Option<(HashValue, HashValue)>,

    /// All siblings in this proof, including the default ones. Siblings near the root are at the
    /// beginning of the vector.
    /// ��֤���е������ֵܽ��ã�����Ĭ�ϵ��ֵܽ��á� ���������ֵܽ���λ�������Ŀ�ͷ��
    siblings: Vec<HashValue>,
}

impl SparseMerkleProof {
    /// Constructs a new `SparseMerkleProof` using leaf and a list of siblings.
    /// ʹ��leaf���ֵ��б���һ���µ�`SparseMerkleProof`��
    pub fn new(leaf: Option<(HashValue, HashValue)>, siblings: Vec<HashValue>) -> Self {
        // The sibling list could be empty in case the Sparse Merkle Tree is empty or has a single
        // element. When it's not empty, the bottom most sibling will never be default, otherwise a
        // leaf and a default sibling should have collapsed to a leaf.
        // ���ϡ��Merkle��Ϊ�ջ���е���Ԫ�أ����ֵ��б����Ϊ�ա� ��������ǿյģ���ô��ײ���ֵ�
        // ����Զ������Ĭ�ϵģ�����Ҷ�Ӻ�Ĭ�ϵ��ֵ�Ӧ���۵���Ҷ�ӡ�
        if let Some(last_sibling) = siblings.last() {
            assert_ne!(*last_sibling, *SPARSE_MERKLE_PLACEHOLDER_HASH);
        }

        SparseMerkleProof { leaf, siblings }
    }

    /// Returns the leaf node in this proof.
    /// ���ش�֤���е�Ҷ�ڵ㡣
    pub fn leaf(&self) -> Option<(HashValue, HashValue)> {
        self.leaf
    }

    /// Returns the list of siblings in this proof.
    /// ���ش�֤���е��ֵ��б�
    pub fn siblings(&self) -> &[HashValue] {
        &self.siblings
    }
}

impl FromProto for SparseMerkleProof {
    type ProtoType = crate::proto::proof::SparseMerkleProof;

    /// Validates `proto_proof` and converts it to `Self` if validation passed.
    /// �����֤ͨ������֤`proto_proof`������ת��Ϊ`Self`��
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
        // ��λͼ�ĵ�һ���ֽڵ�MSB�����ұߵ�1λ������ ���������һ��λ������Ӧ���ֵ��Ƿ�Ĭ�ϵģ����Ǵ�
        // proto_siblings�л�ȡ�ֵܡ� ���򣬴�λ�õ��ֵ���Ĭ�ϵġ�
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
        // �������Ҷ�ӣ����ǽ�����ֵɢ��дΪ64�ֽڵĵ��ֽ����顣 ��������дһ�����ֽ����顣
        if let Some((key, value_hash)) = self.leaf {
            proto_proof.mut_leaf().extend_from_slice(key.as_ref());
            proto_proof
                .mut_leaf()
                .extend_from_slice(value_hash.as_ref());
        }
        // Iterate over all siblings. For each non-default sibling, add to protobuf struct and set
        // the corresponding bit in the bitmap.
        // ���������ֵܽ��á� ����ÿ����Ĭ��ͬ������ӵ�protobuf�ṹ����λͼ��������Ӧ��λ��
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
/// ������֤`SignedTransaction`���������֤���� ����ṹ����һ��``AccumulatorProof`����`LedgerInfo`��
/// `TransactionInfo`����֤����Ҫ��֤`TransactionInfo`�������ȷ�ԣ��Լ�Ӧ����`SignedTransaction`ƥ���`TransactionInfo`����
#[derive(Clone, Debug, Eq, PartialEq, FromProto, IntoProto)]
#[ProtoType(crate::proto::proof::SignedTransactionProof)]
pub struct SignedTransactionProof {
    /// The accumulator proof from ledger info root to leaf that authenticates the hash of the
    /// `TransactionInfo` object.
    /// �ۼ����ӷ�����info root��leaf��֤`TransactionInfo`�����ɢ�С�
    ledger_info_to_transaction_info_proof: AccumulatorProof,

    /// The `TransactionInfo` object at the leaf of the accumulator.
    /// �ۼ���Ҷ���ϵ�`TransactionInfo`����
    transaction_info: TransactionInfo,
}

impl SignedTransactionProof {
    /// Constructs a new `SignedTransactionProof` object using given
    /// `ledger_info_to_transaction_info_proof`.
    /// ʹ�ø�����`ledger_info_to_transaction_info_proof`����һ���µ�`SignedTransactionProof`����
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
    /// ���ش�֤���е�`ledger_info_to_transaction_info_proof`����
    pub fn ledger_info_to_transaction_info_proof(&self) -> &AccumulatorProof {
        &self.ledger_info_to_transaction_info_proof
    }

    /// Returns the `transaction_info` object in this proof.
    /// ���ش�֤���е�`transaction_info`����
    pub fn transaction_info(&self) -> &TransactionInfo {
        &self.transaction_info
    }
}

/// The complete proof used to authenticate the state of an account. This structure consists of the
/// `AccumulatorProof` from `LedgerInfo` to `TransactionInfo`, the `TransactionInfo` object and the
/// `SparseMerkleProof` from state root to the account.
/// ������֤�ʻ�״̬������֤�ݡ� ����ṹ������`LedgerInfo`��`TransactionInfo`��`AccumulatorProof`��
/// `TransactionInfo`����ʹ��ݸ����ʻ���`SparseMerkleProof`��
#[derive(Clone, Debug, Eq, PartialEq, FromProto, IntoProto)]
#[ProtoType(crate::proto::proof::AccountStateProof)]
pub struct AccountStateProof {
    /// The accumulator proof from ledger info root to leaf that authenticates the hash of the
    /// `TransactionInfo` object.
    /// �ۼ����ӷ�����info root��leaf��֤`TransactionInfo`�����ɢ�С�
    ledger_info_to_transaction_info_proof: AccumulatorProof,

    /// The `TransactionInfo` object at the leaf of the accumulator.
    /// �ۼ���Ҷ���ϵ�`TransactionInfo`����
    transaction_info: TransactionInfo,

    /// The sparse merkle proof from state root to the account state.
    /// ��״̬�����ʻ�״̬��ϡ��merkle֤����
    transaction_info_to_account_proof: SparseMerkleProof,
}

impl AccountStateProof {
    /// Constructs a new `AccountStateProof` using given `ledger_info_to_transaction_info_proof`,
    /// `transaction_info` and `transaction_info_to_account_proof`.
    /// ʹ�ø�����`ledger_info_to_transaction_info_proof`��`transaction_info`��
    /// `transaction_info_to_account_proof`����һ���µ�`AccountStateProof`��
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
    /// ���ش�֤���е�`ledger_info_to_transaction_info_proof`����
    pub fn ledger_info_to_transaction_info_proof(&self) -> &AccumulatorProof {
        &self.ledger_info_to_transaction_info_proof
    }

    /// Returns the `transaction_info` object in this proof.
    /// ���ش�֤���е�`transaction_info`����
    pub fn transaction_info(&self) -> &TransactionInfo {
        &self.transaction_info
    }

    /// Returns the `transaction_info_to_account_proof` object in this proof.
    /// ���ش�֤���е�`transaction_info_to_account_proof`����
    pub fn transaction_info_to_account_proof(&self) -> &SparseMerkleProof {
        &self.transaction_info_to_account_proof
    }
}

/// The complete proof used to authenticate a contract event. This structure consists of the
/// `AccumulatorProof` from `LedgerInfo` to `TransactionInfo`, the `TransactionInfo` object and the
/// `AccumulatorProof` from event accumulator root to the event.
/// ������֤��ͬ�¼�������֤�ݡ� ����ṹ������`LedgerInfo`��`TransactionInfo`��`AccumulatorProof`��
/// `TransactionInfo`����ʹ��¼��ۼ��������¼���`AccumulatorProof`��
#[derive(Clone, Debug, Eq, PartialEq, FromProto, IntoProto)]
#[ProtoType(crate::proto::proof::EventProof)]
pub struct EventProof {
    /// The accumulator proof from ledger info root to leaf that authenticates the hash of the
    /// `TransactionInfo` object.
    /// �ۼ����ӷ�����info root��leaf��֤`TransactionInfo`�����ɢ�С�
    ledger_info_to_transaction_info_proof: AccumulatorProof,

    /// The `TransactionInfo` object at the leaf of the accumulator.
    /// �ۼ���Ҷ���ϵ�`TransactionInfo`����
    transaction_info: TransactionInfo,

    /// The accumulator proof from event root to the actual event.
    /// �ۼ������¼���֤����ʵ���¼���
    transaction_info_to_event_proof: AccumulatorProof,
}

impl EventProof {
    /// Constructs a new `EventProof` using given `ledger_info_to_transaction_info_proof`,
    /// `transaction_info` and `transaction_info_to_event_proof`.
    /// ʹ�ø�����`ledger_info_to_transaction_info_proof`����һ���µ�`EventProof`��
    ///  `transaction_info`��`transaction_info_to_event_proof`��
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
    /// λͼָʾ�ĸ��ֵܽڵ���ѹ���ۼ���֤������Ĭ�ϵġ� 1��ʾ��Ĭ��ֵ��0��ʾĬ��ֵ��
    /// LSB��Ӧ���ۼ����ײ����ֵܡ� ����ߵ�1λ��Ӧ���ۼ����������ֵܣ���Ϊ��һ�����Ƿ�Ĭ�ϵġ�
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
