// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::chained_bft::{
    common::{Author, Round},
    consensus_types::{block::Block, quorum_cert::QuorumCert},
};
use crypto::HashValue;
use std::sync::Arc;

mod block_store;
mod block_tree;

use crate::{
    chained_bft::safety::vote_msg::VoteMsgVerificationError,
    state_replication::{ExecutedState, StateComputeResult},
};
pub use block_store::{BlockStore, NeedFetchResult};
use network::protocols::rpc::error::RpcError;
use types::validator_verifier::VerifyError;

#[derive(Debug, PartialEq, Fail)]
/// The possible reasons for failing to retrieve a block by id from a given peer.
/// 无法从给定对等方通过id检索块的可能原因。
#[allow(dead_code)]
#[derive(Clone)]
pub enum BlockRetrievalFailure {
    /// Could not find a given author
    /// 找不到给定的作者
    #[fail(display = "Unknown author: {:?}", author)]
    UnknownAuthor { author: Author },

    /// Any sort of a network failure (should probably have an enum for network failures).
    /// 任何类型的网络故障（应该有网络故障的枚举）。
    #[fail(display = "Network failure: {:?}", msg)]
    NetworkFailure { msg: String },

    /// The remote peer did not recognize the given block id.
    /// 远程对等方无法识别给定的块ID。
    #[fail(display = "Block id {:?} not recognized by the peer", block_id)]
    UnknownBlockId { block_id: HashValue },

    /// Cannot retrieve a block from itself
    /// 无法从自身检索块
    #[fail(display = "Attempt of a self block retrieval.")]
    SelfRetrieval,

    /// The event is not correctly signed.
    /// 该事件未正确签名。
    #[fail(display = "InvalidSignature")]
    InvalidSignature,

    /// The response is not valid: status doesn't match blocks, blocks unable to deserialize etc.
    /// 响应无效：状态与块不匹配，块无法反序列化等。
    #[fail(display = "InvalidResponse")]
    InvalidResponse,
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Fail)]
/// Status after trying to insert a block into the BlockStore
/// 尝试将块插入BlockStore后的状态
pub enum InsertError {
    /// The parent block does not exist, hence not inserting this block
     /// 父块不存在，因此不插入此块
    #[fail(display = "MissingParentBlock")]
    MissingParentBlock(HashValue),
    /// The block hash is invalid
    /// 块哈希无效
    #[fail(display = "InvalidBlockHash")]
    InvalidBlockHash,
    /// The block height is not parent's height + 1.
    /// 块高度不是父高度+ 1。
    #[fail(display = "InvalidBlockHeight")]
    InvalidBlockHeight,
    /// The block round is not greater than that of the parent.
    /// 块轮不大于父轮的轮。
    #[fail(display = "InvalidBlockRound")]
    InvalidBlockRound,
    /// The block's timestamp is not greater than that of the parent.
    /// 块的时间戳不大于父级的时间戳。
    #[fail(display = "InvalidTiemstamp")]
    NonIncreasingTimestamp,
    /// The block is not newer than the root of the tree.
    /// 该块不比树的根更新。
    #[fail(display = "OldBlock")]
    OldBlock,
    /// The event is from unknown an unknown author.
    /// 该事件来自未知的未知作者。
    #[fail(display = "UnknownAuthor")]
    UnknownAuthor,
    /// The event is not correctly signed.
    /// 该事件未正确签名。
    #[fail(display = "InvalidSignature")]
    InvalidSignature,
    /// The external state computer failure.
    /// 外部状态计算机故障。
    #[fail(display = "StateComputeError")]
    StateComputerError,
    /// Block's parent is not certified with the QC carried by the block.
    /// Block的父级未获得块所携带的QC认证。
    #[fail(display = "ParentNotCertified")]
    ParentNotCertified,
    /// State version corresponding to block's parent not found.
    /// 对应于未找到块的父级的状态版本。
    #[fail(display = "ParentVersionNotFound")]
    ParentVersionNotFound,
    /// Some of the block's ancestors could not be retrieved.
    /// 无法检索到某些块的祖先。
    #[fail(display = "AncestorRetrievalError")]
    AncestorRetrievalError,
    #[fail(display = "StorageFailure")]
    StorageFailure,
}

impl From<RpcError> for BlockRetrievalFailure {
    fn from(source: RpcError) -> Self {
        BlockRetrievalFailure::NetworkFailure {
            msg: source.to_string(),
        }
    }
}

impl From<VerifyError> for InsertError {
    fn from(error: VerifyError) -> Self {
        match error {
            VerifyError::UnknownAuthor => InsertError::UnknownAuthor,
            VerifyError::InvalidSignature => InsertError::InvalidSignature,
            VerifyError::TooFewSignatures { .. } => InsertError::InvalidSignature,
            VerifyError::TooManySignatures { .. } => InsertError::InvalidSignature,
        }
    }
}

impl From<VoteMsgVerificationError> for InsertError {
    fn from(_error: VoteMsgVerificationError) -> Self {
        InsertError::InvalidSignature
    }
}

/// Result of the vote processing. The failure case (Verification error) is returned
/// as the Error part of the result.
/// 投票处理的结果。 失败案例（验证错误）作为结果的错误部分返回。
#[derive(Debug, PartialEq)]
pub enum VoteReceptionResult {
    /// The vote has been added but QC has not been formed yet. Return the number of votes for
    /// the given (proposal, execution) pair.
     /// 投票已经加入，但QC尚未形成。 返回给定（提议，执行）对的投票数。
    VoteAdded(usize),
    /// The very same vote message has been processed in past.
    /// 过去曾处理过同样的投票信息。
    DuplicateVote,
    /// This block has been already certified.
    /// 该区块已经过认证。
    OldQuorumCertificate(Arc<QuorumCert>),
    /// This block has just been certified after adding the vote.
    /// 此块在添加投票后刚刚获得认证。
    NewQuorumCertificate(Arc<QuorumCert>),
}

#[derive(Debug, Fail)]
/// Tree query error types.
/// 树查询错误类型。
pub enum BlockTreeError {
    #[fail(display = "Block not found: {:?}", id)]
    BlockNotFound { id: HashValue },
}

impl From<BlockTreeError> for InsertError {
    fn from(error: BlockTreeError) -> InsertError {
        match error {
            BlockTreeError::BlockNotFound { id } => InsertError::MissingParentBlock(id),
        }
    }
}

pub trait BlockReader: Send + Sync {
    type Payload;

    /// Check if a block with the block_id exist in the BlockTree.
     /// 检查BlockTree中是否存在具有block_id的块。
    fn block_exists(&self, block_id: HashValue) -> bool;

    /// Try to get a block with the block_id, return an Arc of it if found.
     /// 尝试使用block_id获取一个块，如果找到则返回一个Arc。
    fn get_block(&self, block_id: HashValue) -> Option<Arc<Block<Self::Payload>>>;

    /// Try to get a state id (HashValue) of the system corresponding to block execution.
     /// 尝试获取与块执行相对应的系统的状态ID（HashValue）。
    fn get_state_for_block(&self, block_id: HashValue) -> Option<ExecutedState>;

    /// Try to get an execution result given the specified block id.
    /// 尝试在给定指定的块ID的情况下获取执行结果。
    fn get_compute_result(&self, block_id: HashValue) -> Option<Arc<StateComputeResult>>;

    /// Get the current root block of the BlockTree.
     /// 获取BlockTree的当前根块。
    fn root(&self) -> Arc<Block<Self::Payload>>;

    fn get_quorum_cert_for_block(&self, block_id: HashValue) -> Option<Arc<QuorumCert>>;

    /// Returns true if a given "ancestor" block is an ancestor of a given "block".
    /// Returns a failure if not all the blocks are present between the block's height and the
    /// parent's height.
    /// 如果给定的“祖先”块是给定“块”的祖先，则返回true。
    /// 如果块的高度和父级的高度之间不存在所有块，则返回失败。
    fn is_ancestor(
        &self,
        ancestor: &Block<Self::Payload>,
        block: &Block<Self::Payload>,
    ) -> Result<bool, BlockTreeError>;

    /// Returns all the blocks between the root and the given block, including the given block
    /// but excluding the root.
    /// In case a given block is not the successor of the root, return None.
    /// For example if a tree is b0 <- b1 <- b2 <- b3, then
    /// path_from_root(b2) -> Some([b2, b1])
    /// path_from_root(b0) -> Some([])
    /// path_from_root(a) -> None
    /// 返回根和给定块之间的所有块，包括给定块但不包括根。
    /// 如果给定块不是根的后继，则返回None。
    /// 例如，如果树是b0 < -  b1 < -  b2 < -  b3，那么
    fn path_from_root(
        &self,
        block: Arc<Block<Self::Payload>>,
    ) -> Option<Vec<Arc<Block<Self::Payload>>>>;

    /// Generates and returns a block with the given parent and payload.
    /// Note that it does not add the block to the tree, just generates it.
    /// The main reason we want this function in the BlockStore is the fact that the signer required
    /// for signing the newly created block is held by the block store.
    /// The function panics in the following cases:
    /// * If the parent or its quorum certificate are not present in the tree,
    /// * If the given round (which is typically calculated by Pacemaker) is not greater than that
    ///   of a parent.
        /// 生成并返回具有给定父级和有效负载的块。
    /// 请注意，它不会将块添加到树中，只是生成它。
    /// 我们在BlockStore中想要这个功能的主要原因是签名新创建的块所需的签名者是由块存储保存的。
    /// 在下列情况下，该功能会出现恐慌：
    /// *如果父树或其仲裁证书不在树中，
    /// *如果给定的一轮（通常由Pacemaker计算）不大于父母的一轮。
    fn create_block(
        &self,
        parent: Arc<Block<Self::Payload>>,
        payload: Self::Payload,
        round: Round,
        timestamp_usecs: u64,
    ) -> Block<Self::Payload>;

    /// Return the certified block with the highest round.
    /// 返回最高圆的认证区块。
    fn highest_certified_block(&self) -> Arc<Block<Self::Payload>>;

    /// Return the quorum certificate with the highest round
    /// 返回最高轮次的仲裁证书
    fn highest_quorum_cert(&self) -> Arc<QuorumCert>;

    /// Return the quorum certificate that carries ledger info with the highest round
     /// 返回带有最高回合分类帐信息的法定人数证书
    fn highest_ledger_info(&self) -> Arc<QuorumCert>;
}
