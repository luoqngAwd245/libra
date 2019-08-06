// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

#![feature(async_await)]
#![deny(missing_docs)]
//! Mempool is used to hold transactions that have been submitted but not yet agreed upon and
//! executed.
//! Mempool用于保存已提交但尚未商定并执行的交易。
//! **Flow**: AC sends transactions into mempool which holds them for a period of time before
//! sending them into consensus.  When a new transaction is added, Mempool shares this transaction
//! with other nodes in the system.  This is a form of “shared mempool” in that transactions between
//! mempools are shared with other validators.  This helps maintain a pseudo global ordering since
//! when a validator receives a transaction from another mempool, it will be ordered when added in
//! the ordered queue of the recipient validator. To reduce network consumption, in “shared mempool”
//! each validator is responsible for delivery of its own transactions (we don't rebroadcast
//! transactions originated on a different peer). Also we only broadcast transactions that have some
//! chance to be included in next block: their sequence number equals to the next sequence number of
//! account or sequential to it. For example, if the current sequence number for an account is 2 and
//! local mempool contains transactions with sequence numbers 2,3,4,7,8, then only transactions 2, 3
//! and 4 will be broadcast.
//! **流程**：AC将交易发送到mempool，在发送它们达成共识之前将其保留一段时间。添加新事务时，
//! Mempool会与系统中的其他节点共享此事务。这是“共享mempool”的一种形式，因为mempools之间的事务
//! 与其他验证器共享。这有助于维护伪全局排序，因为当验证器从另一个mempool接收到一个事务时，它将在
//! 收件人验证器的有序队列中添加时进行排序。为了减少网络消耗，在“共享mempool”中，每个验证器
//! 负责交付自己的事务（我们不会重新发送源自不同对等体的事务）。此外，我们只广播有一些机会被包含
//! 在下一个块中的事务：它们的序列号等于下一个帐号序列或顺序到它。例如，如果帐户的当前序列号为2且
//! 本地mempool包含序列号为2,3,4,7,8的事务，则仅广播事务2,3和4。
//!
//! Consensus pulls transactions from mempool rather than mempool pushing into consensus. This is
//! done so that while consensus is not yet ready for transactions, we keep ordering based on gas
//! and consensus can let transactions build up.  This allows for batching of transactions into a
//! single consensus block as well as prioritizing by gas price. Mempool doesn't  keep track of
//! transactions that were sent to Consensus. On each get_block request, Consensus additionally
//! sends a set of transactions that were pulled from Mempool so far but were not committed yet.
//! This is done so Mempool can be agnostic about different Consensus proposal branches.  Once a
//! transaction is fully executed and written to storage,  Consensus notifies Mempool about it which
//! later drops it from its internal state.
//! 共识从mempool而非mempool推动交易达成共识。 这样做是为了虽然尚未就交易达成共识，但我们会继续根据GAS
//! 进行订购，而共识可以让交易积累起来。这允许将交易批量分配到单个共识块以及按天然气价格划分优先级。
//! Mempool不会跟踪发送给共识的交易。 在每个get_block请求中，Consensus还发送一组到目前为止从Mempool
//! 中提取但尚未提交的事务。 这样做是因为Mempool可以对不同的共识提议分支不可知。 一旦事务完全执行并
//! 写入存储，Consensus就会通知Mempool，后者将其从内部状态中删除。
//! **Internals**: Internally Mempool is modeled as `HashMap<AccountAddress, AccountTransactions>`
//! with various indexes built on top of it. The main index `PriorityIndex` is an ordered queue of
//! transactions that are “ready” to be included in next block(i.e. have sequence number sequential
//! to current for account). This queue is ordered by gas price so that if a client is willing to
//! pay more (than other clients) per unit of execution, then they can enter consensus earlier. Note
//! that although global ordering is maintained by gas price, for a single account, transactions are
//! ordered by sequence number.
//! **内部**：内部Mempool被建模为`HashMap <AccountAddress，AccountTransactions>`
//  在其上构建各种索引。 主索引“PriorityIndex”是一个有序的事务队列，它“准备好”包含在下一个块中
// （即序列号顺序为当前的帐号）。 这个队列按天然气价格排序，这样如果客户愿意为每个执行单位支付更多
// （比其他客户），那么他们可以提前达成共识。 请注意，尽管全球订购由天然气价格维持，但对于单个帐户，
// 交易按序号排序。
//! All transactions that are not ready to be included in the next block are part of separate
//! `ParkingLotIndex`. They will be moved to the ordered queue once some event unblocks them. For
//! example, Mempool has transaction with sequence number 4, while current sequence number for that
//! account is 3. Such transaction is considered to be “non-ready”. Then callback from Consensus
//! notifies that transaction was committed(i.e. transaction 3 was submitted to different node).
//! Such event “unblocks” local transaction and txn4 will be moved to OrderedQueue.
//! 所有未准备好包含在下一个块中的事务都是单独的`ParkingLotIndex`的一部分。 一旦某个事件解除阻塞，
//! 它们将被移动到有序队列。 例如，Mempool具有序列号为4的事务，而该帐户的当前序列号为3.这样的事务
//! 被认为是“非就绪”。 然后来自Consensus的回调通知交易已经提交（即交易3已提交给不同的节点）。 这样
//! 的事件“解锁”本地事务，txn4将被移动到OrderedQueue。
//! Mempool only holds a limited number of transactions to prevent OOMing the system. Additionally
//! there's a limit of number of transactions per account to prevent different abuses/attacks
//! Mempool仅保留有限数量的事务以防止OOMing系统。 此外，每个帐户的交易数量有限，以防止不同的滥用/攻击
//! Transactions in Mempool have two types of expirations: systemTTL and client-specified
//! expiration. Once we hit either of those, the transaction is removed from Mempool. SystemTTL is
//! checked periodically in the background, while the client-specified expiration is checked on
//! every Consensus commit request. We use a separate system TTL to ensure that a transaction won't
//! remain stuck in Mempool forever, even if Consensus doesn't make progress
//! Mempool中的事务有两种类型的过期：systemTTL和客户端指定的过期。 一旦我们点击其中任何一个，该交易
//! 将从Mempool中删除。 在后台定期检查SystemTTL，同时在每个共识提交请求上检查客户端指定的到期时间。
//! 我们使用单独的系统TTL来确保交易不会永远停留在Mempool中，即使共识没有取得进展
pub mod proto;
pub use runtime::MempoolRuntime;

mod core_mempool;
mod mempool_service;
mod runtime;
mod shared_mempool;

// module op counters
use lazy_static::lazy_static;
use metrics::OpMetrics;
lazy_static! {
    static ref OP_COUNTERS: OpMetrics = OpMetrics::new_and_registered("mempool");
}
pub use crate::core_mempool::MempoolAddTransactionStatus;

#[cfg(test)]
mod unit_tests;
