// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    chained_bft::{
        block_storage::BlockReader,
        common::{Payload, Round},
        consensus_types::{block::Block, quorum_cert::QuorumCert},
    },
    counters,
};

use crypto::HashValue;
use serde::{Deserialize, Serialize};
use std::{
    fmt::{Display, Formatter},
    sync::Arc,
};
use types::ledger_info::LedgerInfoWithSignatures;

#[cfg(test)]
#[path = "safety_rules_test.rs"]
mod safety_rules_test;

/// Vote information is returned if a proposal passes the voting rules.
/// The caller might need to persist some of the consensus state before sending out the actual
/// vote message.
/// Vote info also includes the block id that is going to be committed in case this vote gathers
/// QC.
/// 如果提案通过投票规则，则返回投票信息。
/// 在发出实际的投票消息之前，调用者可能需要保持一些共识状态。
/// 投票信息还包括在此投票收集QC时将提交的块ID。
#[derive(Debug, Eq, PartialEq)]
pub struct VoteInfo {
    /// Block id of the proposed block.
    /// 建议块的块ID。
    proposal_id: HashValue,
    /// Round of the proposed block.
    /// 提议块的回合。
    proposal_round: Round,
    /// Consensus state after the voting (e.g., with the updated vote round)
     /// 投票后的共识状态（例如，更新的投票轮次）
    consensus_state: ConsensusState,
    /// The block that should be committed in case this vote gathers QC.
    /// If no block is committed in case the vote gathers QC, return None.
    /// 如果此投票收集QC，应该提交的块。
    /// 如果在投票收集QC的情况下没有提交阻止，则返回None。
    potential_commit_id: Option<HashValue>,
}

impl VoteInfo {
    pub fn proposal_id(&self) -> HashValue {
        self.proposal_id
    }

    pub fn consensus_state(&self) -> &ConsensusState {
        &self.consensus_state
    }

    pub fn potential_commit_id(&self) -> Option<HashValue> {
        self.potential_commit_id
    }
}

#[derive(Debug, Fail, Eq, PartialEq)]
/// Different reasons for proposal rejection
/// 提案拒绝的不同原因
pub enum ProposalReject {
    /// This proposal's round is less than round of preferred block.
    /// Returns the id of the preferred block.
    /// 该提案的轮次小于优选块的轮次。
    /// 返回首选块的id。
    #[fail(
        display = "Proposal's round is lower than round of preferred block at round {:?}",
        preferred_block_round
    )]
    ProposalRoundLowerThenPreferredBlock { preferred_block_round: Round },

    /// This proposal is too old - return last_vote_round
    /// 这个提议太旧了 - 返回last_vote_round
    #[fail(
        display = "Proposal at round {:?} is not newer than the last vote round {:?}",
        proposal_round, last_vote_round
    )]
    OldProposal {
        last_vote_round: Round,
        proposal_round: Round,
    },

    /// Did not find a parent for the proposed block
    #[fail(
        display = "Proposal for {} at round {} error: parent {} not found",
        proposal_id, proposal_round, parent_id
    )]
    ParentNotFound {
        proposal_id: HashValue,
        proposal_round: Round,
        parent_id: HashValue,
    },
}

/// The state required to guarantee safety of the protocol.
/// We need to specify the specific state to be persisted for the recovery of the protocol.
/// (e.g., last vote round and preferred block round).
/// 状态要求保证协议的安全性。
///  我们需要指定要保留的特定状态以恢复协议。
///（例如，最后一轮投票和首选轮次）。
#[derive(Serialize, Default, Deserialize, Debug, Eq, PartialEq, Clone)]
pub struct ConsensusState {
    last_vote_round: Round,
    last_committed_round: Round,

    // A "preferred block" is the two-chain head with the highest block round.
    // We're using the `head` / `tail` terminology for describing the chains of QCs for describing
    // `head` <-- <block>* <-- `tail` chains.

    // A new proposal is voted for only if it's previous block's round is higher or equal to
    // the preferred_block_round.
    // 1) QC chains follow direct parenthood relations because a node must carry a QC to its
    // parent. 2) The "max round" rule applies to the HEAD of the chain and not its TAIL (one
    // does not necessarily apply the other).
    // “优选块”是具有最高块圆的双链头。
    //  我们使用`head` /`tail`术语来描述用于描述`head` < -  <block> * < - `tail`链的QC链。
    //
    // 只有当前一个块的回合高于或等于preferred_block_round时，才会对新提案进行投票。
    // 1）QC链遵循直接父母关系，因为节点必须携带QC到其父节点。
    // 2）“最大回合”规则适用于链的HEAD而不是其TAIL（一个不一定适用于另一个）。
    preferred_block_round: Round,
}

impl Display for ConsensusState {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        write!(
            f,
            "ConsensusState: [\n\
             \tlast_vote_round = {},\n\
             \tlast_committed_round = {},\n\
             \tpreferred_block_round = {}\n\
             ]",
            self.last_vote_round, self.last_committed_round, self.preferred_block_round
        )
    }
}

impl ConsensusState {
    #[cfg(test)]
    pub fn new(
        last_vote_round: Round,
        last_committed_round: Round,
        preferred_block_round: Round,
    ) -> Self {
        Self {
            last_vote_round,
            last_committed_round,
            preferred_block_round,
        }
    }

    /// Returns the last round that was voted on
    /// 返回投票的最后一轮
    pub fn last_vote_round(&self) -> Round {
        self.last_vote_round
    }

    /// Returns the last committed round
    /// 返回最后一个提交的回合
    #[cfg(test)]
    pub fn last_committed_round(&self) -> Round {
        self.last_committed_round
    }

    /// Returns the preferred block round
    /// 返回首选的块轮
    pub fn preferred_block_round(&self) -> Round {
        self.preferred_block_round
    }

    /// Set the last vote round that ensures safety.  If the last vote round increases, return
    /// the new consensus state based with the updated last vote round.  Otherwise, return None.
    /// 设置确保安全的最后一轮投票。 如果最后一轮投票增加，则返回基于更新的最后一轮投票的新共识状态。 否则，返回None。
    fn set_last_vote_round(&mut self, last_vote_round: Round) -> Option<ConsensusState> {
        if last_vote_round <= self.last_vote_round {
            None
        } else {
            self.last_vote_round = last_vote_round;
            counters::LAST_VOTE_ROUND.set(last_vote_round as i64);
            Some(self.clone())
        }
    }

    /// Set the preferred block round
    /// 设置优先块的轮数
    fn set_preferred_block_round(&mut self, preferred_block_round: Round) {
        self.preferred_block_round = preferred_block_round;
        counters::PREFERRED_BLOCK_ROUND.set(preferred_block_round as i64);
    }
}

/// SafetyRules is responsible for two things that are critical for the safety of the consensus:
/// 1) voting rules,
/// 2) commit rules.
/// The only dependency is a block tree, which is queried for ancestry relationships between
/// the blocks and their QCs.
/// SafetyRules is NOT THREAD SAFE (should be protected outside via e.g., RwLock).
/// The commit decisions are returned to the caller as result of learning about a new QuorumCert.
/// SafetyRules负责对共识安全至关重要的两件事：
///  1）投票规则，
/// 2）提交规则。
/// 唯一的依赖是块树，它被查询块和它们的QC之间的祖先关系。
/// SafetyRules不是安全的（应该通过例如RwLock保护在外面）。
/// 作为了解新QuorumCert的结果，提交决策将返回给调用者。
pub struct SafetyRules<T> {
    // To query about the relationships between blocks and QCs.
    block_tree: Arc<dyn BlockReader<Payload = T>>,
    // Keeps the state.
    state: ConsensusState,
}

impl<T: Payload> SafetyRules<T> {
    /// Constructs a new instance of SafetyRules given the BlockTree and ConsensusState.
    /// 在给定BlockTree和ConsensusState的情况下构造SafetyRules的新实例。
    pub fn new(block_tree: Arc<dyn BlockReader<Payload = T>>, state: ConsensusState) -> Self {
        Self { block_tree, state }
    }

    /// Learn about a new quorum certificate. Several things can happen as a result of that:
    /// 1) update the preferred block to a higher value.
    /// 2) commit some blocks.
    /// In case of commits the last committed block is returned.
    /// Requires that all the ancestors of the block are available for at least up to the last
    /// committed block, might panic otherwise.
    /// The update function is invoked whenever a system learns about a potentially high QC.
    /// 了解新的法定人数证书。 由此可能会发生以下几件事：
    ///  1）将首选块更新为更高的值。
    /// 2）提交一些块。
    ///  在提交的情况下，返回最后提交的块。
    ///  要求块的所有祖先至少可用于最后一个提交的块，否则可能会发生混乱。
    /// 只要系统获知可能很高的QC，就会调用更新函数。
    pub fn update(&mut self, qc: &QuorumCert) -> Option<Arc<Block<T>>> {
        // Preferred block rule: choose the highest 2-chain head.
        if let Some(one_chain_head) = self.block_tree.get_block(qc.certified_block_id()) {
            if let Some(two_chain_head) = self.block_tree.get_block(one_chain_head.parent_id()) {
                if two_chain_head.round() >= self.state.preferred_block_round() {
                    self.state.set_preferred_block_round(two_chain_head.round());
                }
            }
        }
        self.process_ledger_info(qc.ledger_info())
    }

    /// Check to see if a processing a new LedgerInfoWithSignatures leads to a commit.  Return a
    /// new committed block if there is one.
    /// 检查处理新的LedgerInfoWithSignatures是否导致提交。 如果有，则返回一个新的已提交块。
    pub fn process_ledger_info(
        &mut self,
        ledger_info: &LedgerInfoWithSignatures,
    ) -> Option<Arc<Block<T>>> {
        // While voting for a block the validators have already calculated the potential commits.
        // In case there are no commits enabled by this ledger info, the committed block id is going
        // to carry some placeholder value (e.g., zero).
        // 在对块进行投票时，验证器已经计算了潜在的提交。
        // 如果该分类帐信息没有启用提交，则提交的块ID将携带一些占位符值（例如，零）。
        let committed_block_id = ledger_info.ledger_info().consensus_block_id();
        if let Some(committed_block) = self.block_tree.get_block(committed_block_id) {
            // We check against the root of the tree instead of last committed round to avoid
            // double commit.
            // Because we only persist the ConsensusState before sending out the vote, it could
            // be lagged behind the reality if we crash between committing and sending the vote.
            // 我们检查树的根而不是最后一次提交，以避免双重提交。
            // 因为我们只是在发出投票之前坚持ConsensusState，如果我们在提交和投票之间崩溃，它可能会落后于现实。
            if committed_block.round() > self.block_tree.root().round() {
                self.state.last_committed_round = committed_block.round();
                return Some(committed_block);
            }
        }
        None
    }

    /// Check if a one-chain at round r+2 causes a commit at round r and return the committed
    /// block at round r if possible
    /// 检查轮r + 2处的单链是否导致轮r处的提交，并且如果可能则返回轮r处的提交块
    pub fn commit_rule_for_certified_block(
        &self,
        one_chain_head: Arc<Block<T>>,
    ) -> Option<Arc<Block<T>>> {
        if let Some(two_chain_head) = self.block_tree.get_block(one_chain_head.parent_id()) {
            if let Some(three_chain_head) = self.block_tree.get_block(two_chain_head.parent_id()) {
                // We're using a so-called 3-chain commit rule: B0 (as well as its prefix)
                // can be committed if there exist certified blocks B1 and B2 that satisfy:
                // 1) B0 <- B1 <- B2 <--
                // 2) round(B0) + 1 = round(B1), and
                // 3) round(B1) + 1 = round(B2).
                // 我们使用的是所谓的3链提交规则：B0（以及它的前缀）
                // 如果存在满足以下条件的认证块B1和B2，则可以提交：
                // 1）B0 < -  B1 < -  B2 < -
                // 2）圆（B0）+ 1 =圆（B1），和
                // 3）圆（B1）+ 1 =圆（B2）。
                if three_chain_head.round() + 1 == two_chain_head.round()
                    && two_chain_head.round() + 1 == one_chain_head.round()
                {
                    return Some(three_chain_head);
                }
            }
        }
        None
    }

    /// Return the highest known committed round
    /// 返回已知最高的承诺回合
    pub fn last_committed_round(&self) -> Round {
        self.state.last_committed_round
    }

    /// Return the new state if the voting round was increased, otherwise ignore.  Increasing the
    /// last vote round is always safe, but can affect liveness and must be increasing
    /// to protect safety.
    /// 如果投票回合增加则返回新状态，否则忽略。 增加最后一轮投票总是安全的，但可能影响到活力，必须增加以保护安全。
    pub fn increase_last_vote_round(&mut self, round: Round) -> Option<ConsensusState> {
        self.state.set_last_vote_round(round)
    }

    /// Clones the up-to-date state of consensus (for monitoring / debugging purposes)
    /// 克隆最新的共识状态（用于监视/调试目的）
    #[allow(dead_code)]
    pub fn consensus_state(&self) -> ConsensusState {
        self.state.clone()
    }

    /// Attempts to vote for a given proposal following the voting rules.
    /// The returned value is then going to be used for either sending the vote or doing nothing.
    /// In case of a vote a cloned consensus state is returned (to be persisted before the vote is
    /// sent).
    /// Requires that all the ancestors of the block are available for at least up to the last
    /// committed block, might panic otherwise.
    /// 尝试根据投票规则对特定提案进行投票。
    /// 然后返回的值将用于发送投票或不执行任何操作。
    ///  在投票的情况下，返回克隆的共识状态（在投票发送之前保持持久）。
    /// 要求块的所有祖先至少可用于最后一个提交的块，否则可能会发生混乱。
    pub fn voting_rule(
        &mut self,
        proposed_block: Arc<Block<T>>,
    ) -> Result<VoteInfo, ProposalReject> {
        if proposed_block.round() <= self.state.last_vote_round() {
            return Err(ProposalReject::OldProposal {
                proposal_round: proposed_block.round(),
                last_vote_round: self.state.last_vote_round(),
            });
        }

        let parent_block = match self.block_tree.get_block(proposed_block.parent_id()) {
            Some(b) => b,
            None => {
                return Err(ProposalReject::ParentNotFound {
                    proposal_id: proposed_block.id(),
                    proposal_round: proposed_block.round(),
                    parent_id: proposed_block.parent_id(),
                });
            }
        };

        let parent_block_round = parent_block.round();
        let respects_preferred_block = parent_block_round >= self.state.preferred_block_round();
        if respects_preferred_block {
            self.state.set_last_vote_round(proposed_block.round());

            // If the vote for the given proposal is gathered into QC, then this QC might eventually
            // commit another block following the rules defined in
            // `commit_rule_for_certified_block()` function.
            // 如果对给定提案的投票被收集到QC中，那么此QC最终可能会按照其中定义的规则提交另一个块
            //  `commit_rule_for_certified_block（）`函数。
            let potential_commit =
                self.commit_rule_for_certified_block(Arc::clone(&proposed_block));
            let potential_commit_id = match potential_commit {
                None => None,
                Some(commit_block) => Some(commit_block.id()),
            };

            Ok(VoteInfo {
                proposal_id: proposed_block.id(),
                proposal_round: proposed_block.round(),
                consensus_state: self.state.clone(),
                potential_commit_id,
            })
        } else {
            Err(ProposalReject::ProposalRoundLowerThenPreferredBlock {
                preferred_block_round: self.state.preferred_block_round(),
            })
        }
    }
}
