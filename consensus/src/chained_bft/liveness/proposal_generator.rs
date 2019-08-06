// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::chained_bft::{common::Round, consensus_types::block::Block};

use crate::{
    chained_bft::{block_storage::BlockReader, common::Payload},
    counters,
    state_replication::TxnManager,
    util::time_service::{wait_if_possible, TimeService, WaitingError, WaitingSuccess},
};
use logger::prelude::*;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[cfg(test)]
#[path = "proposal_generator_test.rs"]
mod proposal_generator_test;

#[derive(Clone, Debug, PartialEq, Fail)]
/// ProposalGeneration logical errors (e.g., given round number is low).
/// ProposalGeneration逻辑错误（例如，给定的轮数很低）。
pub enum ProposalGenerationError {
    /// The round of a certified block we'd like to extend is not lower than the provided round.
    /// 我们想要扩展的认证区块的轮次不低于提供的轮次。
    #[fail(display = "GivenRoundTooLow")]
    GivenRoundTooLow(Round),
    #[fail(display = "TxnRetrievalError")]
    TxnRetrievalError,
    /// Local clock waiting completed, but the timestamp is still not greater than its parent
    /// 本地时钟等待完成，但时间戳仍然不大于其父级
    #[fail(display = "CurrentTimeTooOld")]
    CurrentTimeTooOld,
    /// Local clock waiting would exceed round duration to allow the timestamp to be greater that
    /// its parent
    /// 本地时钟等待将超过循环持续时间以允许时间戳大于其父级
    #[fail(display = "ExceedsMaxRoundDuration")]
    ExceedsMaxRoundDuration,
    /// Already proposed at this round (only a single proposal per round is allowed)
    /// 已经在本轮提出（每轮只允许一个提案）
    #[fail(display = "CurrentTimeTooOld")]
    AlreadyProposed(Round),
}

/// ProposalGenerator is responsible for generating the proposed block on demand: it's typically
/// used by a validator that believes it's a valid candidate for serving as a proposer at a given
/// round.
/// ProposalGenerator is the one choosing the branch to extend:
/// - height is determined as parent.height + 1,
/// - round is given by the caller (typically determined by Pacemaker).
/// The transactions for the proposed block are delivered by TxnManager.
///
/// TxnManager should be aware of the pending transactions in the branch that it is extending,
/// such that it will filter them out to avoid transaction duplication.
/// ProposalGenerator负责按需生成建议的块：它通常由验证器使用，该验证器认为它是在给定轮次中作为提议者的有效候选者。
/// ProposalGenerator是选择要扩展的分支的人：
/// - 身高被确定为parent.height + 1，
/// - 轮询由呼叫者给出（通常由Pacemaker确定）。
/// 建议块的事务由TxnManager提供。
///
/// TxnManager应该知道它正在扩展的分支中的挂起事务，这样它就会将它们过滤掉以避免事务重复。
pub struct ProposalGenerator<T> {
    // Block store is queried both for finding the branch to extend and for generating the
    // proposed block.
    // 查询块存储以查找要扩展的分支和生成建议的块。
    block_store: Arc<dyn BlockReader<Payload = T> + Send + Sync>,
    // Transaction manager is delivering the transactions.
    // 交易管理器在交付交易。
    txn_manager: Arc<dyn TxnManager<Payload = T>>,
    // Time service to generate block timestamps
    // 时间服务生成块时间戳
    time_service: Arc<dyn TimeService>,
    // Max number of transactions to be added to a proposed block.
    // 要添加到建议块的最大事务数。
    max_block_size: u64,
    // Support increasing block timestamps
    // 支持增加块时间戳
    enforce_increasing_timestamps: bool,
    // Last round that a proposal was generated
    // 最后一轮提出了提案
    last_round_generated: Mutex<Round>,
}

impl<T: Payload> ProposalGenerator<T> {
    pub fn new(
        block_store: Arc<dyn BlockReader<Payload = T> + Send + Sync>,
        txn_manager: Arc<dyn TxnManager<Payload = T>>,
        time_service: Arc<dyn TimeService>,
        max_block_size: u64,
        enforce_increasing_timestamps: bool,
    ) -> Self {
        Self {
            block_store,
            txn_manager,
            time_service,
            max_block_size,
            enforce_increasing_timestamps,
            last_round_generated: Mutex::new(0),
        }
    }

    /// The function generates a new proposal block: the returned future is fulfilled when the
    /// payload is delivered by the TxnManager implementation.  At most one proposal can be
    /// generated per round (no proposal equivocation allowed).
    /// Errors returned by the TxnManager implementation are propagated to the caller.
    /// The logic for choosing the branch to extend is as follows:
    /// 1. The function gets the highest head of a one-chain from block tree.
    /// The new proposal must extend hqc_block to ensure optimistic responsiveness.
    /// 2. While the height is ultimately determined as the parent.height + 1, the round is provided
    /// by the caller.
    /// 3. In case a given round is not greater than the calculated parent, return an OldRound
    /// error.
    /// 该函数生成一个新的提议块：当TxnManager实现提供有效负载时，将满足返回的未来。 每轮最多可生成一个提案（不允许提案含糊）。
    ///  TxnManager实现返回的错误将传播给调用者。
    /// 选择扩展分支的逻辑如下：
    /// 1.该函数从块树中获取单链的最高头。
    /// 新提案必须扩展hqc_block以确保乐观的响应能力。
    /// 2.当高度最终确定为parent.height + 1时，轮次由呼叫者提供。
    /// 3.如果给定的一轮不大于计算的父级，则返回OldRound错误。
    pub async fn generate_proposal(
        &self,
        round: Round,
        round_deadline: Instant,
    ) -> Result<Block<T>, ProposalGenerationError> {
        {
            let mut last_round_generated = self.last_round_generated.lock().unwrap();
            if *last_round_generated < round {
                *last_round_generated = round;
            } else {
                return Err(ProposalGenerationError::AlreadyProposed(round));
            }
        }

        let hqc_block = self.block_store.highest_certified_block();
        if hqc_block.round() >= round {
            // The given round is too low.
            return Err(ProposalGenerationError::GivenRoundTooLow(hqc_block.round()));
        }

        // One needs to hold the blocks with the references to the payloads while get_block is
        // being executed: pending blocks vector keeps all the pending ancestors of the extended
        // branch.
        // 在执行get_block时，需要使用对有效负载的引用来保存块：挂起块vector保留扩展分支的所有待定祖先。
        let pending_blocks = match self.block_store.path_from_root(Arc::clone(&hqc_block)) {
            Some(res) => res,
            // In case the whole system moved forward between the check of a round and getting
            // path from root.
            // 如果整个系统在一轮检查和从根获取路径之间向前移动。
            None => {
                return Err(ProposalGenerationError::GivenRoundTooLow(hqc_block.round()));
            }
        };
        //let pending_blocks = self.get_pending_blocks(Arc::clone(&hqc_block));
        // Exclude all the pending transactions: these are all the ancestors of
        // parent (including) up to the root (excluding).
        // 排除所有挂起的事务：这些是父（包括）到根（不包括）的所有祖先。
        let exclude_payload = pending_blocks
            .iter()
            .map(|block| block.get_payload())
            .collect();

        let block_timestamp = {
            if self.enforce_increasing_timestamps {
                match wait_if_possible(
                    self.time_service.as_ref(),
                    Duration::from_micros(hqc_block.timestamp_usecs()),
                    round_deadline,
                )
                .await
                {
                    Ok(waiting_success) => {
                        debug!(
                            "Success with {:?} for getting a valid timestamp for the next proposal",
                            waiting_success
                        );

                        match waiting_success {
                            WaitingSuccess::WaitWasRequired {
                                current_duration_since_epoch,
                                wait_duration,
                            } => {
                                counters::PROPOSAL_SUCCESS_WAIT_MS
                                    .observe(wait_duration.as_millis() as f64);
                                counters::PROPOSAL_WAIT_WAS_REQUIRED_COUNT.inc();
                                current_duration_since_epoch
                            }
                            WaitingSuccess::NoWaitRequired {
                                current_duration_since_epoch,
                                ..
                            } => {
                                counters::PROPOSAL_SUCCESS_WAIT_MS.observe(0.0);
                                counters::PROPOSAL_NO_WAIT_REQUIRED_COUNT.inc();
                                current_duration_since_epoch
                            }
                        }
                    }
                    Err(waiting_error) => {
                        match waiting_error {
                            WaitingError::MaxWaitExceeded => {
                                error!(
                                    "Waiting until parent block timestamp usecs {:?} would exceed the round duration {:?}, hence will not create a proposal for this round",
                                    hqc_block.timestamp_usecs(),
                                    round_deadline);
                                counters::PROPOSAL_FAILURE_WAIT_MS.observe(0.0);
                                counters::PROPOSAL_MAX_WAIT_EXCEEDED_COUNT.inc();
                                return Err(ProposalGenerationError::ExceedsMaxRoundDuration);
                            }
                            WaitingError::WaitFailed {
                                current_duration_since_epoch,
                                wait_duration,
                            } => {
                                error!(
                                    "Even after waiting for {:?}, parent block timestamp usecs {:?} >= current timestamp usecs {:?}, will not create a proposal for this round",
                                    wait_duration,
                                    hqc_block.timestamp_usecs(),
                                    current_duration_since_epoch);
                                counters::PROPOSAL_FAILURE_WAIT_MS
                                    .observe(wait_duration.as_millis() as f64);
                                counters::PROPOSAL_WAIT_FAILED_COUNT.inc();
                                return Err(ProposalGenerationError::CurrentTimeTooOld);
                            }
                        };
                    }
                }
            } else {
                self.time_service.get_current_timestamp()
            }
        };

        let block_store = Arc::clone(&self.block_store);
        match self
            .txn_manager
            .pull_txns(self.max_block_size, exclude_payload)
            .await
        {
            Ok(txns) => Ok(block_store.create_block(
                hqc_block,
                txns,
                round,
                block_timestamp.as_micros() as u64,
            )),
            Err(_) => Err(ProposalGenerationError::TxnRetrievalError),
        }
    }
}
