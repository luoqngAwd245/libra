// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use lazy_static;
use metrics::OpMetrics;
use prometheus::{Histogram, IntCounter, IntGauge};

lazy_static::lazy_static! {
    pub static ref OP_COUNTERS: OpMetrics = OpMetrics::new_and_registered("consensus");
}

lazy_static::lazy_static! {
//////////////////////
// HEALTH COUNTERS 健康计数器
//////////////////////
/// This counter is set to the round of the highest committed block.
/// 此计数器设置为最高承诺块的轮次。
pub static ref LAST_COMMITTED_ROUND: IntGauge = OP_COUNTERS.gauge("last_committed_round");

/// The counter corresponds to the version of the last committed ledger info.
/// 计数器对应于上次提交的分类帐信息的版本。
pub static ref LAST_COMMITTED_VERSION: IntGauge = OP_COUNTERS.gauge("last_committed_version");

/// This counter is set to the round of the highest voted block.
/// 此计数器设置为最高投票区块的轮次。    
pub static ref LAST_VOTE_ROUND: IntGauge = OP_COUNTERS.gauge("last_vote_round");

/// This counter is set to the round of the preferred block (highest 2-chain head).
/// 此计数器设置为首选块的圆（最高2链头）。    
pub static ref PREFERRED_BLOCK_ROUND: IntGauge = OP_COUNTERS.gauge("preferred_block_round");

/// This counter is set to the last round reported by the local pacemaker.
/// 该计数器设置为当地起搏器报告的最后一轮。    
pub static ref CURRENT_ROUND: IntGauge = OP_COUNTERS.gauge("current_round");

/// Count of the block proposals sent by this validator since last restart.
/// 自上次重新启动以来此验证程序发送的块提议的计数。    
pub static ref PROPOSALS_COUNT: IntCounter = OP_COUNTERS.counter("proposals_count");

/// Count of the committed blocks since last restart.
/// 自上次重启以来提交的块的计数。    
pub static ref COMMITTED_BLOCKS_COUNT: IntCounter = OP_COUNTERS.counter("committed_blocks_count");

/// Count of the committed transactions since last restart.
/// 自上次重新启动以来提交的事务的计数。    
pub static ref COMMITTED_TXNS_COUNT: IntCounter = OP_COUNTERS.counter("committed_txns_count");

/// Count of success txns in the blocks committed by this validator since last restart.
/// 自上次重新启动以来此验证程序提交的块中的成功计数txns。    
pub static ref SUCCESS_TXNS_COUNT: IntCounter = OP_COUNTERS.counter("success_txns_count");

/// Count of failed txns in the committed blocks since last restart.
/// FAILED_TXNS_COUNT + SUCCESS_TXN_COUNT == COMMITTED_TXNS_COUNT
pub static ref FAILED_TXNS_COUNT: IntCounter = OP_COUNTERS.counter("failed_txns_count");

//////////////////////
// PACEMAKER COUNTERS 起搏器计数器
//////////////////////
/// Count of the rounds that gathered QC since last restart.
/// 自上次重启以来收集QC的回合计数。    
pub static ref QC_ROUNDS_COUNT: IntCounter = OP_COUNTERS.counter("qc_rounds_count");

/// Count of the timeout rounds since last restart (close to 0 in happy path).
/// 自上次重启以来的超时轮次计数（在快乐路径中接近0）。    
pub static ref TIMEOUT_ROUNDS_COUNT: IntCounter = OP_COUNTERS.counter("timeout_rounds_count");

/// Count the number of timeouts a node experienced since last restart (close to 0 in happy path).
/// This count is different from `TIMEOUT_ROUNDS_COUNT`, because not every time a node has
/// a timeout there is an ultimate decision to move to the next round (it might take multiple
/// timeouts to get the timeout certificate).
/// 计算节点自上次重新启动后经历的超时数（在快乐路径中接近0）。
/// 此计数与“TIMEOUT_ROUNDS_COUNT”不同，因为并非每次节点都有超时时，最终决定转到下一轮（可能需要多次超时才能获得超时证书）。    
pub static ref TIMEOUT_COUNT: IntCounter = OP_COUNTERS.counter("timeout_count");

/// The timeout of the current round.
/// 本轮的超时。    
pub static ref ROUND_TIMEOUT_MS: IntGauge = OP_COUNTERS.gauge("round_timeout_ms");

////////////////////////
// SYNCMANAGER COUNTERS
////////////////////////
/// Count the number of times we invoked state synchronization since last restart.
/// 计算自上次重启以来我们调用状态同步的次数。    
pub static ref STATE_SYNC_COUNT: IntCounter = OP_COUNTERS.counter("state_sync_count");

/// Count the overall number of transactions state synchronizer has retrieved since last restart.
/// Large values mean that a node has been significantly behind and had to replay a lot of txns.
/// 计算状态同步器自上次重新启动以来检索的事务总数。
/// 较大的值意味着节点已经明显落后并且不得不重放大量的txns。    
pub static ref STATE_SYNC_TXN_REPLAYED: IntCounter = OP_COUNTERS.counter("state_sync_txns_replayed");

/// Count the number of block retrieval requests issued since last restart.
/// 计算自上次重启以来发出的块检索请求的数量。
pub static ref BLOCK_RETRIEVAL_COUNT: IntCounter = OP_COUNTERS.counter("block_retrieval_count");

/// Histogram of block retrieval duration.
/// 块检索持续时间的直方图。    
pub static ref BLOCK_RETRIEVAL_DURATION_MS: Histogram = OP_COUNTERS.histogram("block_retrieval_duration_ms");

/// Histogram of state sync duration.
/// 状态同步持续时间的直方图。    
pub static ref STATE_SYNC_DURATION_MS: Histogram = OP_COUNTERS.histogram("state_sync_duration_ms");

//////////////////////
// RECONFIGURATION COUNTERS
//////////////////////
/// Current epoch num
pub static ref EPOCH_NUM: IntGauge = OP_COUNTERS.gauge("epoch_num");
/// The number of validators in the current epoch
/// 当前时期中验证器的数量    
pub static ref CURRENT_EPOCH_NUM_VALIDATORS: IntGauge = OP_COUNTERS.gauge("current_epoch_num_validators");
/// Quorum size in the current epoch
/// 当前时代的法定人数    
pub static ref CURRENT_EPOCH_QUORUM_SIZE: IntGauge = OP_COUNTERS.gauge("current_epoch_quorum_size");


//////////////////////
// BLOCK STORE COUNTERS 区块存储计数器
//////////////////////
/// Counter for the number of blocks in the block tree (including the root).
/// In a "happy path" with no collisions and timeouts, should be equal to 3 or 4.
pub static ref NUM_BLOCKS_IN_TREE: IntGauge = OP_COUNTERS.gauge("num_blocks_in_tree");

//////////////////////
// PERFORMANCE COUNTERS 性能计数器
//////////////////////
/// Histogram of execution time (ms) of non-empty blocks.
/// 非空块的执行时间（ms）的直方图。    
pub static ref BLOCK_EXECUTION_DURATION_MS: Histogram = OP_COUNTERS.histogram("block_execution_duration_ms");

/// Histogram of duration of a commit procedure (the time it takes for the execution / storage to
/// commit a block once we decide to do so).
/// 提交过程的持续时间的直方图（一旦我们决定执行/存储提交块所花费的时间）。    
pub static ref BLOCK_COMMIT_DURATION_MS: Histogram = OP_COUNTERS.histogram("block_commit_duration_ms");

/// Histogram for the number of txns per (committed) blocks.
/// 每个（提交）块的txns数的直方图。    
pub static ref NUM_TXNS_PER_BLOCK: Histogram = OP_COUNTERS.histogram("num_txns_per_block");

/// Histogram of per-transaction execution time (ms) of non-empty blocks
/// (calculated as the overall execution time of a block divided by the number of transactions).
/// 非空块的每事务执行时间（ms）的直方图（计算为块的总执行时间除以事务数）。    
pub static ref TXN_EXECUTION_DURATION_MS: Histogram = OP_COUNTERS.histogram("txn_execution_duration_ms");

/// Histogram of execution time (ms) of empty blocks.
/// 空块的执行时间（ms）的直方图。    
pub static ref EMPTY_BLOCK_EXECUTION_DURATION_MS: Histogram = OP_COUNTERS.histogram("empty_block_execution_duration_ms");

/// Histogram of the time it takes for a block to get committed.
/// Meas
