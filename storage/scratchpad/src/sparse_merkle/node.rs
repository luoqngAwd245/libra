// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! This module defines all kinds of nodes in the Sparse Merkle Tree maintained in scratch pad.
//! There are four kinds of nodes:
//!
//! - An `InternalNode` is a node that has two children. It is same as the internal node in a
//! standard Merkle tree.
//!
//! - A `LeafNode` represents a single account. Similar to what is in storage, a leaf node has a
//! key which is the hash of the account address as well as a value hash which is the hash of the
//! corresponding account blob. The difference is that a `LeafNode` does not always have the value,
//! in the case when the leaf was loaded into memory as part of a non-inclusion proof.
//!
//! - A `SubtreeNode` represents a subtree with one or more leaves. `SubtreeNode`s are generated
//! when we get accounts from storage with proof. It stores the root hash of this subtree.
//!
//! - An `EmptyNode` represents an empty subtree with zero leaf.
//!
//! 该模块定义了暂存区中维护的稀疏Merkle树中的所有节点。这里有四种节点：
//!
//! - 内部节点，拥有两个子节点。 它和标准 merkle树的内部节点一样
//!
//! - 叶子节点 ，代表单个账户。类似于存储中的内容，叶节点具有一个键，该键是帐户地址的哈希值，而值哈希
//! 是相应帐户Blob的哈希值。 不同之处在于，在将叶子作为非包含证明的一部分加载到内存中的情况下，“ LeafNode”
//! 并不总是具有该值。
//!
//! - 子树节点，代表具有一个或多个叶子节点的子树。当我们从存储获取带proof的账户信息的时候，生成子树。
//! 子树节点保存子树的root hash
//!
//! - 空节点，表示没有叶子的空子树

use crypto::{
    hash::{CryptoHash, SPARSE_MERKLE_PLACEHOLDER_HASH},
    HashValue,
};
use std::{
    cell::{Ref, RefCell, RefMut},
    rc::Rc,
};
use types::{
    account_state_blob::AccountStateBlob,
    proof::{SparseMerkleInternalNode, SparseMerkleLeafNode},
};

/// We wrap the node in `RefCell`. The only case when we will mutably borrow the node is when we
/// drop a subtree originated from this node and commit things to storage. In that case we will
/// replace the an `InternalNode` or a `LeafNode` with a `SubtreeNode`.
#[derive(Debug)]
pub struct SparseMerkleNode {
    node: RefCell<Node>,
}

impl SparseMerkleNode {
    /// Constructs a new internal node given two children.
    pub fn new_internal(
        left_child: Rc<SparseMerkleNode>,
        right_child: Rc<SparseMerkleNode>,
    ) -> Self {
        SparseMerkleNode {
            node: RefCell::new(Node::new_internal(left_child, right_child)),
        }
    }

    /// Constructs a new leaf node using given key and value.
    pub fn new_leaf(key: HashValue, value: LeafValue) -> Self {
        SparseMerkleNode {
            node: RefCell::new(Node::new_leaf(key, value)),
        }
    }

    /// Constructs a new subtree node with given root hash.
    pub fn new_subtree(hash: HashValue) -> Self {
        SparseMerkleNode {
            node: RefCell::new(Node::new_subtree(hash)),
        }
    }

    /// Constructs a new empty node.
    pub fn new_empty() -> Self {
        SparseMerkleNode {
            node: RefCell::new(Node::new_empty()),
        }
    }

    /// Immutably borrows the wrapped node.
    pub fn borrow(&self) -> Ref<Node> {
        self.node.borrow()
    }

    /// Mutably borrows the wrapped node.
    pub fn borrow_mut(&self) -> RefMut<Node> {
        self.node.borrow_mut()
    }
}

/// The underlying node is either `InternalNode`, `LeafNode`, `SubtreeNode` or `EmptyNode`.
#[derive(Debug)]
pub enum Node {
    Internal(InternalNode),
    Leaf(LeafNode),
    Subtree(SubtreeNode),
    Empty,
}

impl Node {
    pub fn new_internal(
        left_child: Rc<SparseMerkleNode>,
        right_child: Rc<SparseMerkleNode>,
    ) -> Self {
        Node::Internal(InternalNode::new(left_child, right_child))
    }

    pub fn new_leaf(key: HashValue, value: LeafValue) -> Self {
        Node::Leaf(LeafNode::new(key, value))
    }

    pub fn new_subtree(hash: HashValue) -> Self {
        Node::Subtree(SubtreeNode::new(hash))
    }

    pub fn new_empty() -> Self {
        Node::Empty
    }

    #[cfg(test)]
    pub fn is_subtree(&self) -> bool {
        if let Node::Subtree(_) = self {
            true
        } else {
            false
        }
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        if let Node::Empty = self {
            true
        } else {
            false
        }
    }

    pub fn hash(&self) -> HashValue {
        match self {
            Node::Internal(node) => node.hash(),
            Node::Leaf(node) => node.hash(),
            Node::Subtree(node) => node.hash(),
            Node::Empty => *SPARSE_MERKLE_PLACEHOLDER_HASH,
        }
    }
}

/// An internal node.
#[derive(Debug)]
pub struct InternalNode {
    /// The hash of this internal node which is the root hash of the subtree.
    hash: HashValue,

    /// Pointer to left child.
    left_child: Rc<SparseMerkleNode>,

    /// Pointer to right child.
    right_child: Rc<SparseMerkleNode>,
}

impl InternalNode {
    fn new(left_child: Rc<SparseMerkleNode>, right_child: Rc<SparseMerkleNode>) -> Self {
        match (&*left_child.node.borrow(), &*right_child.node.borrow()) {
            (Node::Subtree(_), Node::Subtree(_)) => {
                panic!("Two subtree children should have been merged into a single subtree node.")
            }
            (Node::Leaf(_), Node::Empty) => {
                panic!("A leaf with an empty sibling should have been merged into a single leaf.")
            }
            (Node::Empty, Node::Leaf(_)) => {
                panic!("A leaf with an empty sibling should have been merged into a single leaf.")
            }
            _ => (),
        }

        let hash =
            SparseMerkleInternalNode::new(left_child.borrow().hash(), right_child.borrow().hash())
                .hash();
        InternalNode {
            hash,
            left_child,
            right_child,
        }
    }

    fn hash(&self) -> HashValue {
        self.hash
    }

    pub fn clone_left_child(&self) -> Rc<SparseMerkleNode> {
        Rc::clone(&self.left_child)
    }

    pub fn clone_right_child(&self) -> Rc<SparseMerkleNode> {
        Rc::clone(&self.right_child)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LeafValue {
    /// The account state blob.
    Blob(AccountStateBlob),

    /// The hash of the blob.
    BlobHash(HashValue),
}

/// A `LeafNode` represents a single account in the Sparse Merkle Tree.
#[derive(Debug)]
pub struct LeafNode {
    /// The key is the hash of the address.
    key: HashValue,

    /// The account blob or its hash. It's possible that we don't know the value here. For example,
    /// this leaf was loaded into memory as part of an non-inclusion proof. In that case we
    /// only know the value's hash.
    value: LeafValue,

    /// The hash of this leaf node which is Hash(key || Hash(value)).
    hash: HashValue,
}

impl LeafNode {
    pub fn new(key: HashValue, value: LeafValue) -> Self {
        let value_hash = match value {
            LeafValue::Blob(ref val) => val.hash(),
            LeafValue::BlobHash(ref val_hash) => *val_hash,
        };
        let hash = SparseMerkleLeafNode::new(key, value_hash).hash();
        LeafNode { key, value, hash }
    }

    pub fn key(&self) -> HashValue {
        self.key
    }

    pub fn value(&self) -> &LeafValue {
        &self.value
    }

    fn hash(&self) -> HashValue {
        self.hash
    }
}

/// A subtree node.
#[derive(Debug)]
pub struct SubtreeNode {
    /// The root hash of the subtree represented by this node.
    hash: HashValue,
}

impl SubtreeNode {
    fn new(hash: HashValue) -> Self {
        assert_ne!(
            hash, *SPARSE_MERKLE_PLACEHOLDER_HASH,
            "A subtree should never be empty."
        );
        SubtreeNode { hash }
    }

    pub fn hash(&self) -> HashValue {
        self.hash
    }
}
