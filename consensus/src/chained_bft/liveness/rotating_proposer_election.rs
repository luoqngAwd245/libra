// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::chained_bft::{
    common::{Payload, Round},
    consensus_types::proposal_info::{ProposalInfo, ProposerInfo},
    liveness::proposer_election::ProposerElection,
};
use channel;
use futures::{Future, FutureExt, SinkExt};
use logger::prelude::*;
use std::pin::Pin;

/// The rotating proposer maps a round to an author according to a round-robin rotation.
/// A fixed proposer strategy loses liveness when the fixed proposer is down. Rotating proposers
/// won't gather quorum certificates to machine loss/byzantine behavior on f/n rounds.
/// 旋转提议器根据循环旋转将圆形映射到作者。
/// 当固定提议者关闭时，固定的提议者策略会失去活力。 轮换提议者不会收集法定证书以进行f / n轮的机器丢失/拜占庭行为。
pub struct RotatingProposer<T, P> {
    // Ordering of proposers to rotate through (all honest replicas must agree on this)
    // 提议者轮换的命令（所有诚实的副本必须对此达成一致）
    proposers: Vec<P>,
    // Number of contiguous rounds (i.e. round numbers increase by 1) a proposer is active
    // in a row
    // 提议者连续处于活动状态的连续回合数（即，回合数增加1）
    contiguous_rounds: u32,
    // Output stream to send the chosen proposals
    // 输出流发送选择的建议
    winning_proposals_sender: channel::Sender<ProposalInfo<T, P>>,
}

impl<T, P: ProposerInfo> RotatingProposer<T, P> {
    /// With only one proposer in the vector, it behaves the same as a fixed proposer strategy.
    /// 在向量中只有一个提议者，它的行为与固定的提议者策略相同。
    pub fn new(
        proposers: Vec<P>,
        contiguous_rounds: u32,
        winning_proposals_sender: channel::Sender<ProposalInfo<T, P>>,
    ) -> Self {
        Self {
            proposers,
            contiguous_rounds,
            winning_proposals_sender,
        }
    }

    fn get_proposer(&self, round: Round) -> P {
        self.proposers
            [((round / u64::from(self.contiguous_rounds)) % self.proposers.len() as u64) as usize]
    }
}

impl<T: Payload, P: ProposerInfo> ProposerElection<T, P> for RotatingProposer<T, P> {
    fn is_valid_proposer(&self, author: P, round: Round) -> Option<P> {
        if self.get_proposer(round).get_author() == author.get_author() {
            Some(author)
        } else {
            None
        }
    }

    fn get_valid_proposers(&self, round: Round) -> Vec<P> {
        vec![self.get_proposer(round)]
    }

    fn process_proposal(
        &self,
        proposal: ProposalInfo<T, P>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        // This is a simple rotating proposer, the proposal is processed in the context of the
        // caller task, no synchronization required because there is no mutable state.
        // 这是一个简单的旋转提议器，提议在调用者任务的上下文中处理，不需要同步，因为没有可变状态。
        let round_author = self.get_proposer(proposal.proposal.round()).get_author();
        if round_author != proposal.proposer_info.get_author() {
            return async {}.boxed();
        }
        let mut sender = self.winning_proposals_sender.clone();
        async move {
            if let Err(e) = sender.send(proposal).await {
                debug!("Error in sending the winning proposal: {:?}", e);
            }
        }
            .boxed()
    }
}
