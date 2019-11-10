// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! In a leader based consensus algorithm, each participant maintains a block tree that looks like
//! the following:
//! ```text
//!  Height      5      6      7      ...
//!
//! Committed -> B5  -> B6  -> B7
//!         |
//!         └--> B5' -> B6' -> B7'
//!                     |
//!                     └----> B7"
//! ```
//! This module implements `BlockTree` that is an in-memory representation of this tree.
//! 在基于领导者的共识算法中，每个参与者都维护一个类似于以下内容的块树：
//!文字
//!  高度5 6 7 ...
//!
//!提交-> B5-> B6-> B7
//!          |
//!          └-> B5'-> B6'-> B7'
//!                     |
//!                      └----> B7”
//!  ```
//!该模块实现了“ BlockTree”，它是该树的内存表示形式。

#[cfg(test)]
mod block_tree_test;

use crypto::HashValue;
use failure::bail_err;
use std::collections::{hash_map, HashMap, HashSet};

/// Each block has a unique identifier that is a `HashValue` computed by consensus. It has exactly
/// one parent and zero or more children.
/// 每个块都有一个唯一的标识符，该标识符是通过共识计算得出的“ HashValue”。 它恰好有一个父母，有零个或多个孩子。
pub trait Block: std::fmt::Debug {
    /// The output of executing this block.
    /// 执行此块的输出。
    type Output;

    /// The signatures on this block.
    /// 此块上的签名。
    type Signature;

    /// Whether consensus has decided to commit this block. This kind of blocks are expected to be
    /// sent to storage very soon, unless execution is lagging behind.
    /// 共识是否已决定实施此阻止措施。 除非执行滞后，否则预计此类块将很快发送到存储。
    fn is_committed(&self) -> bool;

    /// Marks this block as committed.
    /// 将此块标记为已提交。
    fn set_committed(&mut self);

    /// Whether this block has finished execution.
    /// 此块是否已完成执行。
    fn is_executed(&self) -> bool;

    /// Sets the output of this block.
    /// 设置该块的输出。
    fn set_output(&mut self, output: Self::Output);

    /// Sets the signatures for this block.
    /// 设置该块的签名。
    fn set_signature(&mut self, signature: Self::Signature);

    /// The id of this block.
    /// 该块的ID。
    fn id(&self) -> HashValue;

    /// The id of the parent block.
    /// 父块的ID。
    fn parent_id(&self) -> HashValue;

    /// Adds a block as its child.
    fn add_child(&mut self, child_id: HashValue);

    /// The list of children of this block.
    fn children(&self) -> &HashSet<HashValue>;
}

/// The `BlockTree` implementation.
/// `BlockTree`实现。
#[derive(Debug)]
pub struct BlockTree<B> {
    /// A map that keeps track of all existing blocks by their ids.
    /// 按ID跟踪所有现有块的hashmap。
    id_to_block: HashMap<HashValue, B>,

    /// The blocks at the lowest height in the map. B5 and B5' in the following example.
    /// ```text
    /// Committed(B0..4) -> B5  -> B6  -> B7
    ///                |
    ///                └--> B5' -> B6' -> B7'
    ///                            |
    ///                            └----> B7"
    /// ```
    /// 地图中最低高度的方块。 在以下示例中为B5和B5'。
    ///  文字
    ///  提交（B0..4）-> B5-> B6-> B7
    ///                 |
    ///                 └-> B5'-> B6'-> B7'
    ///                             |
    ///                             └----> B7”
    ///  ```
    heads: HashSet<HashValue>,

    /// Id of the last committed block. B4 in the above example.
    /// 最后提交的块的ID。 在以上示例中为B4。
    last_committed_id: HashValue,
}

impl<B> BlockTree<B>
where
    B: Block,
{
    /// Constructs a new `BlockTree`.
    pub fn new(last_committed_id: HashValue) -> Self {
        BlockTree {
            id_to_block: HashMap::new(),
            heads: HashSet::new(),
            last_committed_id,
        }
    }

    /// Adds a new block to the tree.
    pub fn add_block(&mut self, block: B) -> Result<(), AddBlockError<B>> {
        assert!(!self.id_to_block.contains_key(&self.last_committed_id));

        let id = block.id();
        if self.id_to_block.contains_key(&id) {
            bail_err!(AddBlockError::BlockAlreadyExists { block });
        }

        let parent_id = block.parent_id();
        if parent_id == self.last_committed_id {
            assert!(self.heads.insert(id), "Block already existed in heads.");
            self.id_to_block.insert(id, block);
            return Ok(());
        }

        match self.id_to_block.entry(parent_id) {
            hash_map::Entry::Occupied(mut entry) => {
                entry.get_mut().add_child(id);
                assert!(
                    self.id_to_block.insert(id, block).is_none(),
                    "Block {:x} already existed.",
                    id,
                );
            }
            hash_map::Entry::Vacant(_) => bail_err!(AddBlockError::ParentNotFound { block }),
        }

        Ok(())
    }

    /// Returns a reference to a specific block, if it exists in the tree.
    /// 如果树中存在特定块的引用，则返回该块的引用。
    pub fn get_block(&self, id: HashValue) -> Option<&B> {
        self.id_to_block.get(&id)
    }

    /// Returns a mutable reference to a specific block, if it exists in the tree.
    /// 返回对特定块的可变引用（如果它存在于树中）。
    pub fn get_block_mut(&mut self, id: HashValue) -> Option<&mut B> {
        self.id_to_block.get_mut(&id)
    }

    /// Returns id of a block that is ready to be sent to VM for execution (its parent has finished
    /// execution), if such block exists in the tree.
    /// 如果树中存在这样的块，则返回准备发送给VM执行（其父级已完成执行）的块的ID。
    pub fn get_block_to_execute(&mut self) -> Option<HashValue> {
        let mut to_visit: Vec<HashValue> = self.heads.iter().cloned().collect();

        while let Some(id) = to_visit.pop() {
            let block = self
                .id_to_block
                .get(&id)
                .expect("Missing block in id_to_block.");
            if !block.is_executed() {
                return Some(id);
            }
            to_visit.extend(block.children().iter().cloned());
        }

        None
    }

    /// Marks given block and all its uncommitted ancestors as committed. This does not cause these
    /// blocks to be sent to storage immediately.
    /// 将给定的块及其所有未提交的祖先标记为已提交。 这不会导致这些块立即发送到存储。
    pub fn mark_as_committed(
        &mut self,
        id: HashValue,
        signature: B::Signature,
    ) -> Result<(), CommitBlockError> {
        // First put the signatures in the block. Note that if this causes multiple blocks to be
        // marked as committed, only the last one will have the signatures.
        // 首先将签名放在块中。 请注意，如果这导致多个块被标记为已提交，则只有最后一个具有签名。
        match self.id_to_block.get_mut(&id) {
            Some(block) => {
                if block.is_committed() {
                    bail_err!(CommitBlockError::BlockAlreadyMarkedAsCommitted { id });
                } else {
                    block.set_signature(signature);
                }
            }
            None => bail_err!(CommitBlockError::BlockNotFound { id }),
        }

        // Mark the current block as committed. Go to parent block and repeat until a committed
        // block is found, or no more blocks.
        // 将当前块标记为已提交。 转到父块并重复直到找到一个已提交的块，或者没有更多的块。
        let mut current_id = id;
        while let Some(block) = self.id_to_block.get_mut(&current_id) {
            if block.is_committed() {
                break;
            }

            block.set_committed();
            current_id = block.parent_id();
        }

        Ok(())
    }

    /// Removes all blocks in the tree that conflict with committed blocks. Returns a list of
    /// blocks that are ready to be sent to storage (all the committed blocks that have been
    /// executed).
    /// 删除树中与已提交的块冲突的所有块。 返回准备好要发送到存储的块的列表（已执行的所有提交的块）。
    pub fn prune(&mut self) -> Vec<B> {
        let mut blocks_to_store = vec![];

        // First find if there is a committed block in current heads. Since these blocks are at the
        // same height, at most one of them can be committed. If all of them are pending we have
        // nothing to do here.  Otherwise, one of the branches is committed. Throw away the rest of
        // them and advance to the next height.
        // 首先查找当前磁头中是否存在已提交的块。 由于这些块的高度相同，因此最多只能提交其中一个。
        // 如果他们都在等待中，我们在这里无事可做。 否则，将提交分支之一。 扔掉其余的并前进到下一个高度。
        let mut current_heads = self.heads.clone();
        while let Some(committed_head) = self.get_committed_head(&current_heads) {
            assert!(
                current_heads.remove(&committed_head),
                "committed_head should exist.",
            );
            for id in current_heads {
                self.remove_branch(id);
            }

            match self.id_to_block.entry(committed_head) {
                hash_map::Entry::Occupied(entry) => {
                    current_heads = entry.get().children().clone();
                    let current_id = *entry.key();
                    let parent_id = entry.get().parent_id();
                    if entry.get().is_executed() {
                        // If this block has been executed, all its proper ancestors must have
                        // finished execution and present in `blocks_to_store`.
                        // 如果已执行此块，则其所有合适的祖先必须具有完成执行并显示在“ blocks_to_store”中。
                        self.heads = current_heads.clone();
                        self.last_committed_id = current_id;
                        blocks_to_store.push(entry.remove());
                    } else {
                        // The current block has not finished execution. If the parent block does
                        // not exist in the map, that means parent block (also committed) has been
                        // executed and removed. Otherwise self.heads does not need to be changed.
                        // 当前块尚未完成执行。 如果父块在映射中不存在，这意味着已执行并删除了父块（也已提交）。
                        // 否则，无需更改self.heads。
                        if !self.id_to_block.contains_key(&parent_id) {
                            self.heads = HashSet::new();
                            self.heads.insert(current_id);
                        }
                    }
                }
                hash_map::Entry::Vacant(_) => unreachable!("committed_head_id should exist."),
            }
        }

        blocks_to_store
    }

    /// Given a list of heads, returns the committed one if it exists.
    /// 给定一个头列表，如果存在则返回已提交的头。
    fn get_committed_head(&self, heads: &HashSet<HashValue>) -> Option<HashValue> {
        let mut committed_head = None;
        for head in heads {
            let block = self
                .id_to_block
                .get(head)
                .expect("Head should exist in id_to_block.");
            if block.is_committed() {
                assert!(
                    committed_head.is_none(),
                    "Conflicting blocks are both committed.",
                );
                committed_head = Some(*head);
            }
        }
        committed_head
    }

    /// Removes a branch at block `head`.
    /// 移除块“ head”处的分支。
    fn remove_branch(&mut self, head: HashValue) {
        let mut remaining = vec![head];
        while let Some(current_block_id) = remaining.pop() {
            let block = self
                .id_to_block
                .remove(&current_block_id)
                .unwrap_or_else(|| {
                    panic!(
                        "Trying to remove a non-existing block {:x}.",
                        current_block_id,
                    )
                });
            assert!(
                !block.is_committed(),
                "Trying to remove a committed block {:x}.",
                current_block_id,
            );
            remaining.extend(block.children().iter());
        }
    }

    /// Removes the entire subtree at block `id`.
    /// 删除块“ id”处的整个子树。
    pub fn remove_subtree(&mut self, id: HashValue) {
        self.heads.remove(&id);
        self.remove_branch(id);
    }

    /// Resets the block tree with a new `last_committed_id`. This removes all the in-memory
    /// blocks.
    /// 用新的last_committed_id重置块树。 这将删除所有内存块。
    pub fn reset(&mut self, last_committed_id: HashValue) {
        let mut new_block_tree = BlockTree::new(last_committed_id);
        std::mem::swap(self, &mut new_block_tree);
    }
}

/// An error returned by `add_block`. The error contains the block being added so the caller does
/// not lose it.
/// `add_block`返回的错误。 该错误包含要添加的块，因此调用者不会丢失它。
#[derive(Debug, Eq, PartialEq)]
pub enum AddBlockError<B: Block> {
    ParentNotFound { block: B },
    BlockAlreadyExists { block: B },
}

impl<B> AddBlockError<B>
where
    B: Block,
{
    pub fn into_block(self) -> B {
        match self {
            AddBlockError::ParentNotFound { block } => block,
            AddBlockError::BlockAlreadyExists { block } => block,
        }
    }
}

impl<B> std::fmt::Display for AddBlockError<B>
where
    B: Block,
{
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            AddBlockError::ParentNotFound { block } => {
                write!(f, "Parent block {:x} was not found.", block.parent_id())
            }
            AddBlockError::BlockAlreadyExists { block } => {
                write!(f, "Block {:x} already exists.", block.id())
            }
        }
    }
}

/// An error returned by `mark_as_committed`. The error contains id of the block the caller wants
/// to commit.
/// mark_as_committed返回的错误。 该错误包含调用者想要提交的块的ID。
#[derive(Debug, Eq, PartialEq)]
pub enum CommitBlockError {
    BlockNotFound { id: HashValue },
    BlockAlreadyMarkedAsCommitted { id: HashValue },
}

impl std::fmt::Display for CommitBlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            CommitBlockError::BlockNotFound { id } => write!(f, "Block {:x} was not found.", id),
            CommitBlockError::BlockAlreadyMarkedAsCommitted { id } => {
                write!(f, "Block {:x} was already marked as committed.", id)
            }
        }
    }
}
