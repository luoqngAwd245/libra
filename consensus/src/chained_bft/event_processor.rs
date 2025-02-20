// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

#[cfg(test)]
use crate::chained_bft::safety::safety_rules::ConsensusState;
use crate::{
    chained_bft::{
        block_storage::{BlockReader, BlockStore, NeedFetchResult, VoteReceptionResult},
        common::{Author, Payload, Round},
        consensus_types::{
            block::Block,
            proposal_info::{ProposalInfo, ProposerInfo},
            sync_info::SyncInfo,
            timeout_msg::{PacemakerTimeout, TimeoutMsg},
        },
        liveness::{
            pacemaker::{NewRoundEvent, NewRoundReason, Pacemaker},
            proposal_generator::ProposalGenerator,
            proposer_election::ProposerElection,
        },
        network::{
            BlockRetrievalRequest, BlockRetrievalResponse, ChunkRetrievalRequest,
            ConsensusNetworkImpl,
        },
        persistent_storage::PersistentStorage,
        safety::{safety_rules::SafetyRules, vote_msg::VoteMsg},
        sync_manager::{SyncManager, SyncMgrContext},
    },
    counters,
    state_replication::{StateComputer, TxnManager},
    util::time_service::{
        duration_since_epoch, wait_if_possible, TimeService, WaitingError, WaitingSuccess,
    },
};
use crypto::HashValue;
use logger::prelude::*;
use network::proto::BlockRetrievalStatus;
use std::{
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};
use termion::color::*;
use types::ledger_info::LedgerInfoWithSignatures;

/// Result of initial proposal processing
/// - Done(true) indicates that the proposal was sent to the proposer election
/// - NeedFetch means separate task mast be spawned for fetching block
/// - Caller should call fetch_and_process_proposal in separate task when NeedFetch is returned
///初始提案处理的结果
/// NeedFetch意味着生成单独的任务栏以获取块
///当返回NeedFetch时，调用者应该在单独的任务中调用fetch_and_process_proposal
pub enum ProcessProposalResult<T, P> {
    Done(bool),
    NeedFetch(Instant, ProposalInfo<T, P>),
    NeedSync(Instant, ProposalInfo<T, P>),
}

/// Consensus SMR is working in an event based fashion: EventProcessor is responsible for
/// processing the individual events (e.g., process_new_round, process_proposal, process_vote,
/// etc.). It is exposing the async processing functions for each event type.
/// The caller is responsible for running the event loops and driving the execution via some
/// executors.
/// 共识SMR以基于事件的方式工作：EventProcessor负责处理各个事件（例如，process_new_round，process_proposal，
/// process_vote等）。 它为每种事件类型公开了异步处理函数。 调用者负责运行事件循环并通过某些执行程序驱动执行。
pub struct EventProcessor<T, P> {
    author: P,
    block_store: Arc<BlockStore<T>>,
    pacemaker: Arc<dyn Pacemaker>,
    proposer_election: Arc<dyn ProposerElection<T, P> + Send + Sync>,
    proposal_generator: ProposalGenerator<T>,
    safety_rules: Arc<RwLock<SafetyRules<T>>>,
    state_computer: Arc<dyn StateComputer<Payload = T>>,
    txn_manager: Arc<dyn TxnManager<Payload = T>>,
    network: ConsensusNetworkImpl,
    storage: Arc<dyn PersistentStorage<T>>,
    sync_manager: SyncManager<T>,
    time_service: Arc<dyn TimeService>,
    enforce_increasing_timestamps: bool,
}

impl<T: Payload, P: ProposerInfo> EventProcessor<T, P> {
    pub fn new(
        author: P,
        block_store: Arc<BlockStore<T>>,
        pacemaker: Arc<dyn Pacemaker>,
        proposer_election: Arc<dyn ProposerElection<T, P> + Send + Sync>,
        proposal_generator: ProposalGenerator<T>,
        safety_rules: Arc<RwLock<SafetyRules<T>>>,
        state_computer: Arc<dyn StateComputer<Payload = T>>,
        txn_manager: Arc<dyn TxnManager<Payload = T>>,
        network: ConsensusNetworkImpl,
        storage: Arc<dyn PersistentStorage<T>>,
        time_service: Arc<dyn TimeService>,
        enforce_increasing_timestamps: bool,
    ) -> Self {
        let sync_manager = SyncManager::new(
            Arc::clone(&block_store),
            Arc::clone(&storage),
            network.clone(),
            Arc::clone(&state_computer),
        );
        Self {
            author,
            block_store,
            pacemaker,
            proposer_election,
            proposal_generator,
            safety_rules,
            state_computer,
            txn_manager,
            network,
            storage,
            sync_manager,
            time_service,
            enforce_increasing_timestamps,
        }
    }

    /// Leader:
    ///
    /// This event is triggered by a new quorum certificate at the previous round or a
    /// timeout certificate at the previous round.  In either case, if this replica is the new
    /// proposer for this round, it is ready to propose and guarantee that it can create a proposal
    /// that all honest replicas can vote for.  While this method should only be invoked at most
    /// once per round, we ensure that only at most one proposal can get generated per round to
    /// avoid accidental equivocation of proposals.
    ///
    /// Replica:
    ///
    /// Do nothing
    ///
    /// leader：
    ///
    /// 此事件由上一轮的新仲裁证书或上一轮的超时证书触发。 在任何一种情况下，如果这个副本是本轮的新提议者，
    /// 它就可以提议并保证它可以创建一个所有诚实副本都可以投票的提案。 虽然此方法每轮最多只能调用一次，
    /// 但我们确保每轮最多只能生成一个提案，以避免意外模糊提案。
    ///
    /// Replica：
    ///
    /// 不做任何事情
    ///
    pub async fn process_new_round_event(&self, new_round_event: NewRoundEvent) {
        debug!("Processing {}", new_round_event);
        counters::CURRENT_ROUND.set(new_round_event.round as i64);
        counters::ROUND_TIMEOUT_MS.set(new_round_event.timeout.as_millis() as i64);
        match new_round_event.reason {
            NewRoundReason::QCReady => {
                counters::QC_ROUNDS_COUNT.inc();
            }
            NewRoundReason::Timeout { .. } => {
                counters::TIMEOUT_ROUNDS_COUNT.inc();
            }
        };
        let proposer_info = match self
            .proposer_election
            .is_valid_proposer(self.author, new_round_event.round)
        {
            Some(pi) => pi,
            None => {
                return;
            }
        };

        // Proposal generator will ensure that at most one proposal is generated per round
        // 提案生成器将确保每轮最多生成一个提案
        let proposal = match self
            .proposal_generator
            .generate_proposal(
                new_round_event.round,
                self.pacemaker.current_round_deadline(),
            )
            .await
        {
            Err(e) => {
                error!("Error while generating proposal: {:?}", e);
                return;
            }
            Ok(proposal) => proposal,
        };
        let mut network = self.network.clone();
        debug!("Propose {}", proposal);
        let timeout_certificate = match new_round_event.reason {
            NewRoundReason::Timeout { cert } => Some(cert),
            _ => None,
        };
        let sync_info = SyncInfo::new(
            (*proposal.quorum_cert()).clone(),
            (*self.block_store.highest_ledger_info()).clone(),
            timeout_certificate,
        );
        network
            .broadcast_proposal(ProposalInfo {
                proposal,
                proposer_info,
                sync_info,
            })
            .await;
        counters::PROPOSALS_COUNT.inc();
    }

    /// The function is responsible for processing the incoming proposals and the Quorum
    /// Certificate. 1. commit to the committed state the new QC carries
    /// 2. fetch all the blocks from the committed state to the QC
    /// 3. forwarding the proposals to the ProposerElection queue,
    /// which is going to eventually trigger one winning proposal per round
    /// (to be processed via a separate function).
    /// The reason for separating `process_proposal` from `process_winning_proposal` is to
    /// (a) asynchronously prefetch dependencies and
    /// (b) allow the proposer election to choose one proposal out of many.
    /// 该功能负责处理传入的提议和Quorum证书。
    /// 1.提交新QC携带的提交状态
    /// 2.从提交状态获取所有块到QC
    /// 3.将提议转发到ProposerElection队列，最终将每轮触发一个获胜提议（通过 一个单独的功能）。
    /// 将`process_proposal`与`process_winning_proposal`分开的原因是：
    /// （a）异步预取依赖关系和
    /// （b）允许提议人选举从众多提案中选出一项提案。
    pub async fn process_proposal(
        &self,
        proposal: ProposalInfo<T, P>,
    ) -> ProcessProposalResult<T, P> {
        debug!("Receive proposal {}", proposal);
        // Pacemaker is going to be updated with all the proposal certificates later,
        // but it's known that the pacemaker's round is not going to decrease so we can already
        // filter out the proposals from old rounds.
	// Pacemaker将在稍后更新所有提案证书，
         //但是众所周知，起搏器的回合不会减少，所以我们已经可以了
         //过滤旧轮次的提案。
        let current_round = self.pacemaker.current_round();
        if proposal.proposal.round() < self.pacemaker.current_round() {
            warn!(
                "Proposal {} is ignored because its round {} != current round {}",
                proposal,
                proposal.proposal.round(),
                current_round
            );
            return ProcessProposalResult::Done(false);
        }
        if self
            .proposer_election
            .is_valid_proposer(proposal.proposer_info, proposal.proposal.round())
            .is_none()
        {
            warn!(
                "Proposer {} for block {} is not a valid proposer for this round",
                proposal.proposal.author(),
                proposal.proposal
            );
            return ProcessProposalResult::Done(false);
        }

        let deadline = self.pacemaker.current_round_deadline();
        if let Some(committed_block_id) = proposal
            .sync_info
            .highest_ledger_info()
            .committed_block_id()
        {
            if self.block_store.need_sync_for_quorum_cert(
                committed_block_id,
                proposal.sync_info.highest_ledger_info(),
            ) {
                return ProcessProposalResult::NeedSync(deadline, proposal);
            }
        } else {
            warn!("Highest ledger info {} has no committed block", proposal);
            return ProcessProposalResult::Done(false);
        }

        match self
            .block_store
            .need_fetch_for_quorum_cert(proposal.proposal.quorum_cert())
        {
            NeedFetchResult::NeedFetch => {
                return ProcessProposalResult::NeedFetch(deadline, proposal)
            }
            NeedFetchResult::QCRoundBeforeRoot => {
                warn!("Proposal {} has a highest quorum certificate with round older than root round {}", proposal, self.block_store.root().round());
                return ProcessProposalResult::Done(false);
            }
            NeedFetchResult::QCBlockExist => {
                if let Err(e) = self
                    .block_store
                    .insert_single_quorum_cert(proposal.proposal.quorum_cert().clone())
                    .await
                {
                    warn!(
                        "Quorum certificate for proposal {} could not be inserted to the block store: {:?}",
                        proposal, e
                    );
                    return ProcessProposalResult::Done(false);
                }
            }
            NeedFetchResult::QCAlreadyExist => (),
        }

        self.finish_proposal_processing(proposal).await
    }

    /// Finish proposal processing: note that multiple tasks can execute this function in parallel
    /// so be careful with the updates. The safest thing to do is to pass the proposal further
    /// to the proposal election.
    /// This function is invoked when all the dependencies for the given proposal are ready.
    /// 完成提议处理：请注意，多个任务可以并行执行此功能，因此请小心更新。 最安全的做法是将提案进一步提交给提案选举。
    /// 当给定提议的所有依赖项都准备就绪时，将调用此函数。
    async fn finish_proposal_processing(
        &self,
        proposal: ProposalInfo<T, P>,
    ) -> ProcessProposalResult<T, P> {
        let qc = proposal.proposal.quorum_cert();
        self.pacemaker
            .process_certificates(
                qc.certified_block_round(),
                proposal.sync_info.highest_timeout_certificate(),
            )
            .await;

        let current_round = self.pacemaker.current_round();
        if self.pacemaker.current_round() != proposal.proposal.round() {
            warn!(
                "Proposal {} is ignored because its round {} != current round {}",
                proposal,
                proposal.proposal.round(),
                current_round
            );
            return ProcessProposalResult::Done(false);
        }

        self.proposer_election.process_proposal(proposal).await;
        ProcessProposalResult::Done(true)
    }

    /// Fetches and completes processing proposal in dedicated task
    /// 在专用任务中获取并完成处理建议
    pub async fn fetch_and_process_proposal(
        &self,
        deadline: Instant,
        proposal: ProposalInfo<T, P>,
    ) {
        if let Err(e) = self
            .sync_manager
            .fetch_quorum_cert(
                proposal.proposal.quorum_cert().clone(),
                proposal.proposer_info.get_author(),
                deadline,
            )
            .await
        {
            warn!(
                "Quorum certificate for proposal {} could not be added to the block store: {:?}",
                proposal, e
            );
            return;
        }
        self.finish_proposal_processing(proposal).await;
    }

    /// Takes mutable reference to avoid race with other processing and perform state
    /// synchronization, then completes processing proposal in dedicated task
    /// 采用可变引用以避免与其他处理竞争并执行状态同步，然后在专用任务中完成处理提议
    ///
    pub async fn sync_and_process_proposal(
        &mut self,
        deadline: Instant,
        proposal: ProposalInfo<T, P>,
    ) {
        // check if we still need sync 检查我们是否还需要同步
        if let Err(e) = self
            .sync_manager
            .sync_to(
                deadline,
                SyncMgrContext::new(&proposal.sync_info, proposal.proposer_info.get_author()),
            )
            .await
        {
            warn!(
                "Quorum certificate for proposal {} could not be added to the block store: {:?}",
                proposal, e
            );
            return;
        }
        self.finish_proposal_processing(proposal).await;
    }

    /// Upon receiving TimeoutMsg, ensure that any branches with higher quorum certificates are
    /// populated to this replica prior to processing the pacemaker timeout.  This ensures that when
    /// a pacemaker timeout certificate is formed with 2f+1 timeouts, the next proposer will be
    /// able to chain a proposal block to a highest quorum certificate such that all honest replicas
    /// can vote for it.
    /// 收到TimeoutMsg后，请确保在处理起搏器超时之前，将具有较高仲裁证书的任何分支填充到此副本。
    /// 这确保了当用2f + 1超时形成起搏器超时证书时，下一个提议者将能够将提议块链接到最高法定人数证书，
    /// 使得所有诚实的副本都可以投票。

    pub async fn process_timeout_msg(&mut self, timeout_msg: TimeoutMsg) {
        debug!(
            "Received a new round msg for round {} from {}",
            timeout_msg.pacemaker_timeout().round(),
            timeout_msg.author().short_str()
        );
        let current_highest_quorum_cert_round = self
            .block_store
            .highest_quorum_cert()
            .certified_block_round();
        let new_round_highest_quorum_cert_round = timeout_msg
            .sync_info()
            .highest_quorum_cert()
            .certified_block_round();

        if current_highest_quorum_cert_round < new_round_highest_quorum_cert_round {
            // The timeout message carries a QC higher than what this node has seen before:
            // run state synchronization.
            // 超时消息的QC高于此节点之前看到的QC：运行状态同步。
            let deadline = self.pacemaker.current_round_deadline();
            match self
                .sync_manager
                .sync_to(
                    deadline,
                    SyncMgrContext::new(
                        timeout_msg.sync_info(),
                        timeout_msg.author(),
                    ),
                )
                .await
                {
                    Ok(()) => debug!(
                        "Successfully added new highest quorum certificate at round {} from old round {}",
                        new_round_highest_quorum_cert_round, current_highest_quorum_cert_round
                    ),
                    Err(e) => warn!(
                        "Unable to insert new highest quorum certificate {} from old round {} due to {:?}",
                        timeout_msg.sync_info().highest_quorum_cert(),
                        current_highest_quorum_cert_round,
                        e
                    ),
                }
        }
        self.pacemaker
            .process_remote_timeout(timeout_msg.pacemaker_timeout().clone())
            .await;
    }

    pub async fn process_sync_info_msg(&mut self, sync_info: SyncInfo) {
        debug!("Received a sync info msg: {}", sync_info);
    }

    /// The replica stops voting for this round and saves its consensus state.  Voting is halted
    /// to ensure that the next proposer can make a proposal that can be voted on by all replicas.
    /// Saving the consensus state ensures that on restart, the replicas will not waste time
    /// on previous rounds.
    /// 副本停止投票，并保存其共识状态。 投票停止以确保下一个提议者能够提出可由所有复制品投票的提案。
    /// 保存共识状态可确保在重新启动时，副本不会在前几轮中浪费时间。
    pub async fn process_outgoing_pacemaker_timeout(&self, round: Round) -> Option<TimeoutMsg> {
        // Stop voting at this round, persist the consensus state to support restarting from
        // a recent round (i.e. > the last vote round)  and then send the highest quorum
        // certificate known
	//在此轮停止投票，坚持共识状态以支持重启
         //最近一轮（即>最后一轮投票），然后发送最高法定人数
         //证书已知
        let consensus_state = self
            .safety_rules
            .write()
            .unwrap()
            .increase_last_vote_round(round);
        if let Some(consensus_state) = consensus_state {
            if let Err(e) = self.storage.save_consensus_state(consensus_state) {
                error!("Failed to persist consensus state after increasing the last vote round due to {:?}", e);
                return None;
            }
        }

        let last_vote_round = self
            .safety_rules
            .read()
            .unwrap()
            .consensus_state()
            .last_vote_round();
        warn!(
            "Round {} timed out and {}, expected round proposer was {:?}, broadcasting new round to all replicas",
            round,
            if last_vote_round == round { "already executed and voted at this round" } else { "will never vote at this round" },
            self.proposer_election.get_valid_proposers(round),
        );

        Some(TimeoutMsg::new(
            SyncInfo::new(
                self.block_store.highest_quorum_cert().as_ref().clone(),
                self.block_store.highest_ledger_info().as_ref().clone(),
                None,
            ),
            PacemakerTimeout::new(round, self.block_store.signer()),
            self.block_store.signer(),
        ))
    }

    /// This function processes a proposal that was chosen as a representative of its round:
    /// 1. Add it to a block store.
    /// 2. Try to vote for it following the safety rules.
    /// 3. In case a validator chooses to vote, send the vote to the representatives at the next
    /// position.
    /// 此功能处理被选为其轮次代表的提案：
    /// 1.将其添加到块存储。
    /// 2 .按照安全规则尝试投票。
    /// 3.如果验证人选择投票，则将投票发送给下一个位置的代表。

    pub async fn process_winning_proposal(&self, proposal: ProposalInfo<T, P>) {
        let qc = proposal.proposal.quorum_cert();
        let update_res = self.safety_rules.write().unwrap().update(qc);
        if let Some(new_commit) = update_res {
            let finality_proof = qc.ledger_info().clone();
            self.process_commit(new_commit, finality_proof).await;
        }

        if let Some(time_to_receival) = duration_since_epoch()
            .checked_sub(Duration::from_micros(proposal.proposal.timestamp_usecs()))
        {
            counters::CREATION_TO_RECEIVAL_MS.observe(time_to_receival.as_millis() as f64);
        }
        let block = match self
            .sync_manager
            .execute_and_insert_block(proposal.proposal)
            .await
        {
            Err(e) => {
                debug!(
                    "Block proposal could not be added to the block store: {:?}",
                    e
                );
                return;
            }
            Ok(block) => block,
        };

        // Checking pacemaker round again, because multiple proposal can now race
        // during async block retrieval
	//再次检查心脏起搏器，因为多个提案现在可以竞赛
         //在异步块检索期间
        if self.pacemaker.current_round() != block.round() {
            debug!(
                "Skip voting for winning proposal {} rejected because round is incorrect. Pacemaker: {}, proposal: {}",
                block,
                self.pacemaker.current_round(),
                block.round()
            );
            return;
        }

        let current_round_deadline = self.pacemaker.current_round_deadline();
        if self.enforce_increasing_timestamps {
            match wait_if_possible(
                self.time_service.as_ref(),
                Duration::from_micros(block.timestamp_usecs()),
                current_round_deadline,
            )
            .await
            {
                Ok(waiting_success) => {
                    debug!("Success with {:?} for being able to vote", waiting_success);

                    match waiting_success {
                        WaitingSuccess::WaitWasRequired { wait_duration, .. } => {
                            counters::VOTE_SUCCESS_WAIT_MS
                                .observe(wait_duration.as_millis() as f64);
                            counters::VOTE_WAIT_WAS_REQUIRED_COUNT.inc();
                        }
                        WaitingSuccess::NoWaitRequired { .. } => {
                            counters::VOTE_SUCCESS_WAIT_MS.observe(0.0);
                            counters::VOTE_NO_WAIT_REQUIRED_COUNT.inc();
                        }
                    }
                }
                Err(waiting_error) => {
                    match waiting_error {
                        WaitingError::MaxWaitExceeded => {
                            error!(
                                "Waiting until proposal block timestamp usecs {:?} would exceed the round duration {:?}, hence will not vote for this round",
                                block.timestamp_usecs(),
                                current_round_deadline);
                            counters::VOTE_FAILURE_WAIT_MS.observe(0.0);
                            counters::VOTE_MAX_WAIT_EXCEEDED_COUNT.inc();
                            return;
                        }
                        WaitingError::WaitFailed {
                            current_duration_since_epoch,
                            wait_duration,
                        } => {
                            error!(
                                "Even after waiting for {:?}, proposal block timestamp usecs {:?} >= current timestamp usecs {:?}, will not vote for this round",
                                wait_duration,
                                block.timestamp_usecs(),
                                current_duration_since_epoch);
                            counters::VOTE_FAILURE_WAIT_MS
                                .observe(wait_duration.as_millis() as f64);
                            counters::VOTE_WAIT_FAILED_COUNT.inc();
                            return;
                        }
                    };
                }
            }
        }

        let vote_info = match self
            .safety_rules
            .write()
            .unwrap()
            .voting_rule(Arc::clone(&block))
        {
            Err(e) => {
                debug!("{}Rejected{} {}: {:?}", Fg(Red), Fg(Reset), block, e);
                return;
            }
            Ok(vote_info) => vote_info,
        };
        if let Err(e) = self
            .storage
            .save_consensus_state(vote_info.consensus_state().clone())
        {
            debug!("Fail to persist consensus state: {:?}", e);
            return;
        }
        let proposal_id = vote_info.proposal_id();
        let executed_state = self
            .block_store
            .get_state_for_block(proposal_id)
            .expect("Block proposal: no execution state found for inserted block.");

        let ledger_info_placeholder = self
            .block_store
            .ledger_info_placeholder(vote_info.potential_commit_id());
        let vote_msg = VoteMsg::new(
            proposal_id,
            executed_state,
            block.round(),
            self.author.get_author(),
            ledger_info_placeholder,
            self.block_store.signer(),
        );

        let recipients: Vec<Author> = self
            .proposer_election
            .get_valid_proposers(block.round() + 1)
            .iter()
            .map(ProposerInfo::get_author)
            .collect();
        debug!(
            "{}Voted for{} {}, potential commit {}",
            Fg(Green),
            Fg(Reset),
            block,
            vote_info
                .potential_commit_id()
                .unwrap_or_else(HashValue::zero)
        );
        self.network.send_vote(vote_msg, recipients).await;
    }

    /// Upon new vote:
    /// 1. Filter out votes for rounds that should not be processed by this validator (to avoid
    /// potential attacks).
    /// 2. Add the vote to the store and check whether it finishes a QC.
    /// 3. Once the QC successfully formed, notify the Pacemaker.
    ///  新投票时：
    /// 1.筛选出不应由此验证程序处理的轮次的投票（以避免潜在的攻击）。
    /// 2.将投票添加到商店并检查它是否完成了质量控制。
    /// 3.质量控制成功后，通知心脏起搏器。
    #[allow(clippy::collapsible_if)] // Collapsing here would make if look ugly
    pub async fn process_vote(&self, vote: VoteMsg, quorum_size: usize) {
        // Check whether this validator is a valid recipient of the vote.
        // 检查此验证器是否是投票的有效收件人。
        let next_round = vote.round() + 1;
        if self
            .proposer_election
            .is_valid_proposer(self.author, next_round)
            .is_none()
        {
            debug!(
                "Received {}, but I am not a valid proposer for round {}, ignore.",
                vote, next_round
            );
            security_log(SecurityEvent::InvalidConsensusVote)
                .error("InvalidProposer")
                .data(vote)
                .data(next_round)
                .log();
            return;
        }

        let deadline = self.pacemaker.current_round_deadline();
        // TODO [Reconfiguration] Verify epoch of the vote message.
        // Add the vote and check whether it completes a new QC.
        // TODO [重新配置]验证投票消息的时期。
        //  添加投票并检查是否完成了新的质量控制。
        match self
            .block_store
            .insert_vote(vote.clone(), quorum_size)
            .await
        {
            VoteReceptionResult::DuplicateVote => {
                // This should not happen in general.
                // 这不应该发生。
                security_log(SecurityEvent::DuplicateConsensusVote)
                    .error(VoteReceptionResult::DuplicateVote)
                    .data(vote)
                    .log();
            }
            VoteReceptionResult::NewQuorumCertificate(qc) => {
                if self.block_store.need_fetch_for_quorum_cert(&qc) == NeedFetchResult::NeedFetch {
                    if let Err(e) = self
                        .sync_manager
                        .fetch_quorum_cert(qc.as_ref().clone(), vote.author(), deadline)
                        .await
                    {
                        error!("Error syncing to qc {}: {:?}", qc, e);
                        return;
                    }
                } else {
                    if let Err(e) = self
                        .block_store
                        .insert_single_quorum_cert(qc.as_ref().clone())
                        .await
                    {
                        error!("Error inserting qc {}: {:?}", qc, e);
                        return;
                    }
                }
                // Notify the Pacemaker about the new QC round.
                // 向Pacemaker通报新的QC回合。
                self.pacemaker
                    .process_certificates(vote.round(), None)
                    .await;
            }
            // nothing interesting with votes arriving for the QC that has been formed
            // 没有任何有趣的选票到达已经形成的QC
            _ => {}
        };
    }

    /// Upon new commit:
    /// 1. Notify state computer with the finality proof.
    /// 2. After the state is finalized, update the txn manager with the status of the committed
    /// transactions.
    /// 3. Prune the tree.
    /// 新提交时：
    /// 1.通过最终证明通知国家计算机。
    /// 2.状态完成后，使用已提交事务的状态更新txn管理器。
    /// 3.修剪树。
    async fn process_commit(
        &self,
        committed_block: Arc<Block<T>>,
        finality_proof: LedgerInfoWithSignatures,
    ) {
        // Verify that the ledger info is indeed for the block we're planning to
        // commit.
        // 验证分类帐信息确实是我们计划提交的块。
        assert_eq!(
            finality_proof.ledger_info().consensus_block_id(),
            committed_block.id()
        );

        // Update the pacemaker with the highest committed round so that on the next round
        // duration it calculates, the initial round index is reset
        // 使用最高承诺回合更新起搏器，以便在计算的下一轮持续时间内重置初始回合索引
        self.pacemaker
            .update_highest_committed_round(committed_block.round());

        if let Err(e) = self.state_computer.commit(finality_proof).await {
            // We assume that state computer cannot enter an inconsistent state that might
            // violate safety of the protocol. Specifically, an executor service is going to panic
            // if it fails to persist the commit requests, which would crash the whole process
            // including consensus.
            // 我们假设状态计算机不能进入可能违反协议安全性的不一致状态。 具体来说，如果执行者服务
            // 无法持久保存提交请求，那么它将会出现恐慌，这会导致包括共识在内的整个过程崩溃。
            error!(
                "Failed to persist commit, mempool will not be notified: {:?}",
                e
            );
            return;
        }
        // At this moment the new state is persisted and we can notify the clients.
        // Multiple blocks might be committed at once: notify about all the transactions in the
        // path from the old root to the new root.
        // 此时新状态持续存在，我们可以通知客户。 可以一次提交多个块：通知从旧根到新根的路径中的所有事务。
        for committed in self
            .block_store
            .path_from_root(Arc::clone(&committed_block))
            .unwrap_or_else(Vec::new)
        {
            if let Some(time_to_commit) = duration_since_epoch()
                .checked_sub(Duration::from_micros(committed.timestamp_usecs()))
            {
                counters::CREATION_TO_COMMIT_MS.observe(time_to_commit.as_millis() as f64);
            }
            let compute_result = self
                .block_store
                .get_compute_result(committed.id())
                .expect("Compute result of a pending block is unknown");
            if let Err(e) = self
                .txn_manager
                .commit_txns(
                    committed.get_payload(),
                    compute_result.as_ref(),
                    committed.timestamp_usecs(),
                )
                .await
            {
                error!("Failed to notify mempool: {:?}", e);
            }
        }
        counters::LAST_COMMITTED_ROUND.set(committed_block.round() as i64);
        debug!("{}Committed{} {}", Fg(Blue), Fg(Reset), *committed_block);
        self.block_store.prune_tree(committed_block.id()).await;
    }

    /// Retrieve a n chained blocks from the block store starting from
    /// an initial parent id, returning with <n (as many as possible) if
    /// id or its ancestors can not be found.
    ///
    /// The current version of the function is not really async, but keeping it this way for
    /// future possible changes.
    ///
    /// 从初始父ID开始从块存储中检索n个链式块，如果找不到id或其祖先，则返回<n（尽可能多）。
    ///
    /// 该函数的当前版本并不是真正的异步，而是保持这种方式以适应未来可能的变化。
    pub async fn process_block_retrieval(&self, request: BlockRetrievalRequest<T>) {
        let mut blocks = vec![];
        let mut status = BlockRetrievalStatus::SUCCEEDED;
        let mut id = request.block_id;
        while (blocks.len() as u64) < request.num_blocks {
            if let Some(block) = self.block_store.get_block(id) {
                id = block.parent_id();
                blocks.push(Block::clone(block.as_ref()));
            } else {
                status = BlockRetrievalStatus::NOT_ENOUGH_BLOCKS;
                break;
            }
        }

        if blocks.is_empty() {
            status = BlockRetrievalStatus::ID_NOT_FOUND;
        }

        if let Err(e) = request
            .response_sender
            .send(BlockRetrievalResponse { status, blocks })
        {
            error!("Failed to return the requested block: {:?}", e);
        }
    }

    /// Retrieve the chunk from storage and send it back.
    /// We'll also try to add the QuorumCert into block store if it's for a existing block and
    /// potentially commit.
    /// 从存储中检索块并将其发回。
    /// 我们还将尝试将QuorumCert添加到块存储中，如果它是针对现有块并且可能提交的话。

    pub async fn process_chunk_retrieval(&self, request: ChunkRetrievalRequest) {
        if self
            .block_store
            .block_exists(request.target.certified_block_id())
            && self
                .block_store
                .get_quorum_cert_for_block(request.target.certified_block_id())
                .is_none()
        {
            if let Err(e) = self
                .block_store
                .insert_single_quorum_cert(request.target.clone())
                .await
            {
                error!(
                    "Failed to insert QuorumCert {} from ChunkRetrievalRequest: {}",
                    request.target, e
                );
                return;
            }
            let update_res = self
                .safety_rules
                .write()
                .expect("[state synchronizer handler] unable to lock safety rules")
                .process_ledger_info(&request.target.ledger_info());

            if let Some(block) = update_res {
                self.process_commit(block, request.target.ledger_info().clone())
                    .await;
            }
        }

        let target_version = request.target.ledger_info().ledger_info().version();

        let response = self
            .sync_manager
            .get_chunk(request.start_version, target_version, request.batch_size)
            .await;

        if let Err(e) = request.response_sender.send(response) {
            error!("Failed to return the requested chunk: {:?}", e);
        }
    }

    /// Inspect the current consensus state.
    ///  检查目前的共识状态。
    #[cfg(test)]
    pub fn consensus_state(&self) -> ConsensusState {
        self.safety_rules.read().unwrap().consensus_state()
    }
}
