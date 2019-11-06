// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::chained_bft::{common::Round, consensus_types::proposal_info::ProposalInfo};
use futures::Future;
use std::pin::Pin;

/// ProposerElection incorporates the logic of choosing a leader among multiple candidates.
/// We are open to a possibility for having multiple proposers per round, the ultimate choice
/// of a proposal is exposed by the election protocol via the stream of proposals.
/// ProposerElection包含了在多个候选人中选择领导者的逻辑。
/// 我们对每轮提出多个提议者的可能性持开放态度，提案的最终选择通过选举协议通过提案流程公开。
pub trait ProposerElection<T, P> {
    /// If a given author is a valid candidate for being a proposer, generate the info,
    /// otherwise return None.
    /// Note that this function is synchronous.
    /// 如果给定作者是作为提议者的有效候选者，则生成信息，否则返回None。
    ///
    /// 请注意，此功能是同步的。
    fn is_valid_proposer(&self, author: P, round: Round) -> Option<P>;

    /// Return all the possible valid proposers for a given round (this information can be
    /// used by e.g., voters for choosing the destinations for sending their votes to).
    /// 返回给定轮次的所有可能的有效提议者（例如，选民可以使用该信息来选择发送他们的选票的目的地）。
    fn get_valid_proposers(&self, round: Round) -> Vec<P>;

    /// Notify proposer election about a new proposal. The function doesn't return any information:
    /// proposer election is going to notify the client about the chosen proposal via a dedicated
    /// channel (to be passed in constructor).
    /// 通知提议者选举新提案。 该函数不返回任何信息：
    /// 提议者选举将通过专用渠道（在构造函数中传递）通知客户关于所选提案。
    fn process_proposal(
        &self,
        proposal: ProposalInfo<T, P>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

impl<T: Payload, P: ProposerInfo> IntoProto for ProposalInfo<T, P> {
    type ProtoType = ProtoProposal;

    fn into_proto(self) -> Self::ProtoType {
        let mut proto = Self::ProtoType::new();
        let (block, proposer, hli) = (self.proposal, self.proposer_info, self.highest_ledger_info);
        proto.set_proposed_block(block.into_proto());
        proto.set_proposer(
            to_vec_named(&proposer)
                .expect("fail to serialize proposer info")
                .into(),
        );
        if let Some(tc) = self.timeout_certificate {
            proto.set_timeout_quorum_cert(tc.into_proto());
        }
        proto.set_highest_ledger_info(hli.into_proto());
        proto
    }
}

impl<T: Payload, P: ProposerInfo> FromProto for ProposalInfo<T, P> {
    type ProtoType = ProtoProposal;

    fn from_proto(mut object: Self::ProtoType) -> Result<Self> {
        let proposal = Block::<T>::from_proto(object.take_proposed_block())?;
        let proposer_info = from_slice(object.get_proposer())?;
        let highest_ledger_info = QuorumCert::from_proto(object.take_highest_ledger_info())?;
        let timeout_certificate = if let Some(tc) = object.timeout_quorum_cert.into_option() {
            Some(PacemakerTimeoutCertificate::from_proto(tc)?)
        } else {
            None
        };
        Ok(ProposalInfo {
            proposal,
            proposer_info,
            timeout_certificate,
            highest_ledger_info,
        })
    }
}
