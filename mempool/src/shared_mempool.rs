// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    core_mempool::{CoreMempool, TimelineState},
    OP_COUNTERS,
};
use config::config::{MempoolConfig, NodeConfig};
use failure::prelude::*;
use futures::sync::mpsc::UnboundedSender;
use futures_preview::{
    channel::mpsc,
    compat::{Future01CompatExt, Stream01CompatExt},
    future::join_all,
    FutureExt, SinkExt, Stream, StreamExt, TryFutureExt, TryStreamExt,
};
use logger::prelude::*;
use network::{
    proto::MempoolSyncMsg,
    validator_network::{Event, MempoolNetworkEvents, MempoolNetworkSender},
};
use proto_conv::{FromProto, IntoProto};
use std::{
    collections::HashMap,
    ops::Deref,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use storage_client::StorageRead;
use tokio::{
    runtime::{Builder, Runtime, TaskExecutor},
    timer::Interval,
};
use types::{transaction::SignedTransaction, PeerId};
use vm_validator::vm_validator::{get_account_state, TransactionValidation};

/// state of last sync with peer
/// `timeline_id` is position in log of ready transactions
/// `is_alive` - is connection healthy
/// 连同peer的上一次同步状态
/// `timeline_id`在就绪交易日志中的位置
/// `is_alive` - 连接的健康
#[derive(Clone)]
struct PeerSyncState {
    timeline_id: u64,
    is_alive: bool,
}

type PeerInfo = HashMap<PeerId, PeerSyncState>;

/// Outbound peer syncing event emitted by [`IntervalStream`].
#[derive(Debug)]
pub(crate) struct SyncEvent;

type IntervalStream = Pin<Box<dyn Stream<Item = Result<SyncEvent>> + Send + 'static>>;

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum SharedMempoolNotification {
    Sync,
    PeerStateChange,
    NewTransactions,
}

/// Struct that owns all dependencies required by shared mempool routines
struct SharedMempool<V>
where
    V: TransactionValidation + 'static,
{
    mempool: Arc<Mutex<CoreMempool>>,
    network_sender: MempoolNetworkSender,
    config: MempoolConfig,
    storage_read_client: Arc<dyn StorageRead>,
    validator: Arc<V>,
    peer_info: Arc<Mutex<PeerInfo>>,
    subscribers: Vec<UnboundedSender<SharedMempoolNotification>>,
}

// TODO(gzh): Cannot derive `Clone`.
// Issue: https://github.com/rust-lang/rust/issues/26925
impl<V> Clone for SharedMempool<V>
where
    V: TransactionValidation + 'static,
{
    fn clone(&self) -> Self {
        Self {
            mempool: Arc::clone(&self.mempool),
            network_sender: self.network_sender.clone(),
            config: self.config.clone(),
            storage_read_client: Arc::clone(&self.storage_read_client),
            validator: Arc::clone(&self.validator),
            peer_info: self.peer_info.clone(),
            subscribers: self.subscribers.clone(),
        }
    }
}
// 通知订阅者
fn notify_subscribers(
    event: SharedMempoolNotification,
    subscribers: &[UnboundedSender<SharedMempoolNotification>],
) {
    for subscriber in subscribers {
        let _ = subscriber.unbounded_send(event);
    }
}

fn default_timer(tick_ms: u64) -> IntervalStream {
    Interval::new_interval(Duration::from_millis(tick_ms))
        .compat()
        .map_ok(|_| SyncEvent)
        .map_err(|_| format_err!("[shared mempool] timer tick error"))
        .boxed()
}

/// new peer discovery handler
/// adds new entry to `peer_info`
fn new_peer(peer_info: &Mutex<PeerInfo>, peer_id: PeerId) {
    peer_info
        .lock()
        .expect("[shared mempool] failed to acquire peer_info lock")
        .entry(peer_id)
        .or_insert(PeerSyncState {
            timeline_id: 0,
            is_alive: true,
        })
        .is_alive = true;
}

/// lost peer handler. Marks connection as dead
fn lost_peer(peer_info: &Mutex<PeerInfo>, peer_id: PeerId) {
    if let Some(state) = peer_info
        .lock()
        .expect("[shared mempool] failed to acquire peer_info lock")
        .get_mut(&peer_id)
    {
        state.is_alive = false;
    }
}

/// sync routine
/// used to periodically broadcast ready to go transactions to peers
/// 同步例程，用于定期向其他验证节点广播准备好的交易
///
async fn sync_with_peers<'a>(
    peer_info: &'a Mutex<PeerInfo>,
    mempool: &'a Mutex<CoreMempool>,
    network_sender: &'a mut MempoolNetworkSender,
    batch_size: usize,
) {
    // Clone the underlying peer_info map and use this to sync and collect
    // state updates. We do this instead of holding the lock for the whole
    // function since that would hold the lock across await points which is bad.
    // 克隆基础peer_info映射并使用它来同步和收集状态更新。 我们这样做而不是整个函数保持锁定，
    // 因为那样会保持锁通过坏掉的await点。
    let peer_info_copy = peer_info
        .lock()
        .expect("[shared mempool] failed to acquire peer_info lock")
        .deref()
        .clone();

    let mut state_updates = vec![];

    for (peer_id, peer_state) in peer_info_copy.into_iter() {
        if peer_state.is_alive {
            let timeline_id = peer_state.timeline_id;

            //读取从timeline_id开始的batch_size条交易

            let (transactions, new_timeline_id) = mempool
                .lock()
                .expect("[shared mempool] failed to acquire mempool lock")
                .read_timeline(timeline_id, batch_size);
             //交易不为空
            if !transactions.is_empty() {
                OP_COUNTERS.inc_by("smp.sync_with_peers", transactions.len());
                let mut msg = MempoolSyncMsg::new();
                msg.set_peer_id(peer_id.into());
                msg.set_transactions(
                    transactions
                        .into_iter()
                        .map(IntoProto::into_proto)
                        .collect(),
                );

                debug!(
                    "MempoolNetworkSender.send_to peer {} msg {:?}",
                    peer_id, msg
                );
                // Since this is a direct-send, this will only error if the network
                // module has unexpectedly crashed or shutdown.
                // 由于这是直接发送，因此只有在网络模块意外崩溃或关闭时才会出错。
                network_sender
                    .send_to(peer_id, msg)
                    .await
                    .expect("[shared mempool] failed to direct-send mempool sync message");
            }

            state_updates.push((peer_id, new_timeline_id));
        }
    }

    // Lock the shared peer_info and apply state updates.
    let mut peer_info = peer_info
        .lock()
        .expect("[shared mempool] failed to acquire peer_info lock");
    for (peer_id, new_timeline_id) in state_updates {
        peer_info
            .entry(peer_id)
            .and_modify(|t| t.timeline_id = new_timeline_id);
    }
}

/// used to validate incoming transactions and add them to local Mempool
/// 用于验证传入的事务并将它们添加到本地Mempool
async fn process_incoming_transactions<V>(
    smp: SharedMempool<V>,
    peer_id: PeerId,
    transactions: Vec<SignedTransaction>,
    mut completion_channel: mpsc::Sender<()>,
) where
    V: TransactionValidation,
{
    let account_states = join_all(
        transactions
            .iter()
            .map(|t| get_account_state(smp.storage_read_client.clone(), t.sender())),
    )
    .await;

    // eagerly filter out transactions that were already committed
    let transactions: Vec<_> = transactions
        .into_iter()
        .enumerate()
        .filter_map(|(idx, t)| {
            if let Ok((sequence_number, balance)) = account_states[idx] {
                if t.sequence_number() >= sequence_number {
                    return Some((t, sequence_number, balance));
                }
            }
            None
        })
        .collect();

    let validations = join_all(
        transactions
            .iter()
            .map(|t| smp.validator.validate_transaction(t.0.clone()).compat()),
    )
    .await;

    {
        let mut mempool = smp
            .mempool
            .lock()
            .expect("[shared mempool] failed to acquire mempool lock");

        for (idx, (transaction, sequence_number, balance)) in transactions.into_iter().enumerate() {
            if let Ok(None) = validations[idx] {
                let gas_cost = transaction.max_gas_amount();
                let insertion_result = mempool.add_txn(
                    transaction,
                    gas_cost,
                    sequence_number,
                    balance,
                    TimelineState::NonQualified,
                );
                OP_COUNTERS.inc(&format!(
                    "smp.transactions.status.{:?}.{:?}",
                    insertion_result.code, peer_id
                ));
            } else {
                OP_COUNTERS.inc(&format!(
                    "smp.transactions.status.validation_failed.{:?}",
                    peer_id
                ));
            }
        }
    }
    notify_subscribers(SharedMempoolNotification::NewTransactions, &smp.subscribers);
    if let Err(e) = completion_channel.send(()).await {
        error!("[smp] error in sync task completion {:?}", e);
    }
}

/// This task handles [`SyncEvent`], which is periodically emitted for us to
/// broadcast ready to go transactions to peers.
/// 此任务处理[`SyncEvent`]，它会定期发送给我们广播准备好的事务到其他验证节点。
///
async fn outbound_sync_task<V>(smp: SharedMempool<V>, mut interval: IntervalStream)
where
    V: TransactionValidation,
{
    let peer_info = smp.peer_info;
    let mempool = smp.mempool;
    let mut network_sender = smp.network_sender;
    let batch_size = smp.config.shared_mempool_batch_size;
    let subscribers = smp.subscribers;

    while let Some(sync_event) = interval.next().await {
        trace!("SyncEvent: {:?}", sync_event);
        match sync_event {
            Ok(_) => {
                sync_with_peers(&peer_info, &mempool, &mut network_sender, batch_size).await;
                notify_subscribers(SharedMempoolNotification::Sync, &subscribers);
            }
            Err(e) => {
                error!("Error in outbound_sync_task timer interval: {:?}", e);
                break;
            }
        }
    }

    crit!("SharedMempool outbound_sync_task terminated");
}

/// This task handles inbound network events.
/// 此任务处理入站网络事件
async fn inbound_network_task<V>(
    smp: SharedMempool<V>,
    executor: TaskExecutor,
    mut network_events: MempoolNetworkEvents,
) where
    V: TransactionValidation,
{
    let peer_info = smp.peer_info.clone();
    let subscribers = smp.subscribers.clone();
    let mut workers_available = smp.config.shared_mempool_max_concurrent_inbound_syncs;
    let (sender, mut receiver) = mpsc::channel(workers_available);

    // 直接处理NewPeer / LostPeer事件，因为它们不是异步的，我们不想缓冲它们或让它们重新排序。
    // 入站直接发送消息放置在有界FuturesUnordered队列中，并允许同时执行。 .buffer_unordered（）
    // 也能正确处理背压，所以如果mempool很慢，背压会传播到网络。
    while let Some(event) = network_events.next().await {
        trace!("SharedMempoolEvent::NetworkEvent::{:?}", event);
        match event {
            Ok(network_event) => match network_event {
                Event::NewPeer(peer_id) => {
                    OP_COUNTERS.inc("smp.event.new_peer");
                    new_peer(&peer_info, peer_id);
                    notify_subscribers(SharedMempoolNotification::PeerStateChange, &subscribers);
                }
                Event::LostPeer(peer_id) => {
                    OP_COUNTERS.inc("smp.event.lost_peer");
                    lost_peer(&peer_info, peer_id);
                    notify_subscribers(SharedMempoolNotification::PeerStateChange, &subscribers);
                }
                Event::Message((peer_id, mut msg)) => {
                    if workers_available == 0 {
                        receiver.next().await;
                    } else {
                        workers_available -= 1;
                    }
                    OP_COUNTERS.inc("smp.event.message");
                    let transactions: Vec<_> = msg
                        .take_transactions()
                        .into_iter()
                        .filter_map(|txn| match SignedTransaction::from_proto(txn) {
                            Ok(t) => Some(t),
                            Err(e) => {
                                security_log(SecurityEvent::InvalidTransactionMP)
                                    .error(&e)
                                    .data(&msg)
                                    .log();
                                None
                            }
                        })
                        .collect();
                    OP_COUNTERS.inc_by(
                        &format!("smp.transactions.received.{:?}", peer_id),
                        transactions.len(),
                    );
                    let future = process_incoming_transactions(
                        smp.clone(),
                        peer_id,
                        transactions,
                        sender.clone(),
                    );
                    executor.spawn(future.boxed().unit_error().compat());
                }
                _ => {
                    security_log(SecurityEvent::InvalidNetworkEventMP)
                        .error("UnexpectedNetworkEvent")
                        .data(&network_event)
                        .log();
                    unreachable!("Unexpected network event")
                }
            },
            Err(e) => {
                security_log(SecurityEvent::InvalidNetworkEventMP)
                    .error(&e)
                    .log();
            }
        }
    }
    crit!("SharedMempool inbound_network_task terminated");
}

/// GC all expired transactions by SystemTTL
/// 用SystemTTL回收所有过期的交易
async fn gc_task(mempool: Arc<Mutex<CoreMempool>>, gc_interval_ms: u64) {
    let mut interval = Interval::new_interval(Duration::from_millis(gc_interval_ms)).compat();
    while let Some(res) = interval.next().await {
        match res {
            Ok(_) => {
                mempool
                    .lock()
                    .expect("[shared mempool] failed to acquire mempool lock")
                    .gc_by_system_ttl();
            }
            Err(e) => {
                error!("Error in gc_task timer interval: {:?}", e);
                break;
            }
        }
    }

    crit!("SharedMempool gc_task terminated");
}

/// bootstrap of SharedMempool
/// creates separate Tokio Runtime that runs following routines:
///   - outbound_sync_task (task that periodically broadcasts transactions to peers)
///   - inbound_network_task (task that handles inbound mempool messages and network events)
///   - gc_task (task that performs GC of all expired transactions by SystemTTL)
/// SharedMempool的引导
///创建以下例程运行的单独的Tokio Runtime：
///  -  outbound_sync_task（定期向对等方广播事务的任务）
///  -  inbound_network_task（处理入站mempool消息和网络事件的任务）
///  -  gc_task（由SystemTTL执行所有过期事务的GC的任务）
pub(crate) fn start_shared_mempool<V>(
    config: &NodeConfig,
    mempool: Arc<Mutex<CoreMempool>>,
    network_sender: MempoolNetworkSender,
    network_events: MempoolNetworkEvents,
    storage_read_client: Arc<dyn StorageRead>,
    validator: Arc<V>,
    subscribers: Vec<UnboundedSender<SharedMempoolNotification>>,
    timer: Option<IntervalStream>,
) -> Runtime
where
    V: TransactionValidation + 'static,
{
    let runtime = Builder::new()
        .name_prefix("shared-mem-")
        .build()
        .expect("[shared mempool] failed to create runtime");
    let executor = runtime.executor();

    let peer_info = Arc::new(Mutex::new(PeerInfo::new()));

    let smp = SharedMempool {
        mempool: mempool.clone(),
        config: config.mempool.clone(),
        network_sender,
        storage_read_client,
        validator,
        peer_info,
        subscribers,
    };

    let interval =
        timer.unwrap_or_else(|| default_timer(config.mempool.shared_mempool_tick_interval_ms));
    //同步事务给其他节点
    executor.spawn(
        outbound_sync_task(smp.clone(), interval)
            .boxed()
            .unit_error()
            .compat(),
    );

    //接收事务来自其他节点
    executor.spawn(
        inbound_network_task(smp, executor.clone(), network_events)
            .boxed()
            .unit_error()
            .compat(),
    );

    //定时清理过期交易
    executor.spawn(
        gc_task(mempool, config.mempool.system_transaction_gc_interval_ms)
            .boxed()
            .unit_error()
            .compat(),
    );

    runtime
}
