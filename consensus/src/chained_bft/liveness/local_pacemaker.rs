// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{
    chained_bft::{
        common::Round,
        consensus_types::timeout_msg::{PacemakerTimeout, PacemakerTimeoutCertificate},
        liveness::{
            pacemaker::{NewRoundEvent, NewRoundReason, Pacemaker},
            pacemaker_timeout_manager::{HighestTimeoutCertificates, PacemakerTimeoutManager},
        },
        persistent_storage::PersistentLivenessStorage,
    },
    counters,
    util::time_service::{SendTask, TimeService},
};
use channel;
use futures::{Future, FutureExt, SinkExt, StreamExt, TryFutureExt};
use logger::prelude::*;
use std::{
    cmp::{self, max},
    pin::Pin,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};
use termion::color::*;
use tokio::runtime::TaskExecutor;

/// Determines the maximum round duration based on the round difference between the current
/// round and the committed round
/// 根据当前回合与承诺回合之间的舍入差异确定最大回合持续时间
pub trait PacemakerTimeInterval: Send + Sync + 'static {
    /// Use the index of the round after the highest quorum certificate to commit a block and
    /// return the duration for this round
    ///
    /// Round indices start at 0 (round index = 0 is the first round after the round that led
    /// to the highest committed round).  Given that round r is the highest round to commit a
    /// block, then round index 0 is round r+1.  Note that for genesis does not follow the
    /// 3-chain rule for commits, so round 1 has round index 0.  For example, if one wants
    /// to calculate the round duration of round 6 and the highest committed round is 3 (meaning
    /// the highest round to commit a block is round 5, then the round index is 0.
     ///
    /// 使用最高法定人数证书后的回合索引提交块并返回此回合的持续时间
    ///
    /// 圆形指数从0开始（圆形指数= 0是导致最高承诺回合的回合后的第一轮）。 鉴于round r是提交块的最高轮，
    /// 则round round 0是round r + 1。 请注意，对于genesis，不遵循提交的3链规则，因此第1轮具有圆形索引0.
    /// 例如，如果想要计算第6轮的循环持续时间并且最高承诺轮数为3（意味着最高轮到 提交一个块是第5轮，然后圆形索引是0。

    fn get_round_duration(&self, round_index_after_committed_qc: usize) -> Duration;
}

/// Round durations increase exponentially
/// Basically time interval is base * mul^power
/// Where power=max(rounds_since_qc, max_exponent)
/// 回合持续时间呈指数增长
/// 基本上时间间隔是基础* mul ^功率
/// 其中power = max（rounds_since_qc，max_exponent）
#[derive(Clone)]
pub struct ExponentialTimeInterval {
    // Initial time interval duration after a successful quorum commit.
     // 成功提交仲裁后的初始时间间隔持续时间。
    base_ms: u64,
    // By how much we increase interval every time
     // 我们每次增加间隔的程度
    exponent_base: f64,
    // Maximum time interval won't exceed base * mul^max_pow.
    // Theoretically, setting it means
    // that we rely on synchrony assumptions when the known max messaging delay is
    // max_interval.  Alternatively, we can consider using max_interval to meet partial synchrony
    // assumptions where while delta is unknown, it is <= max_interval.
    // 最大时间间隔不会超过base * mul ^ max_pow。
    // 从理论上讲，设置它意味着当已知的最大消息传递延迟是max_interval时，我们依赖于同步假设。
    // 或者，我们可以考虑使用max_interval来满足部分同步假设，其中当delta未知时，它是<= max_interval。
    max_exponent: usize,
}

impl ExponentialTimeInterval {
    #[cfg(test)]
    pub fn fixed(duration: Duration) -> Self {
        Self::new(duration, 1.0, 0)
    }

    pub fn new(base: Duration, exponent_base: f64, max_exponent: usize) -> Self {
        assert!(
            max_exponent < 32,
            "max_exponent for PacemakerTimeInterval should be <32"
        );
        assert!(
            exponent_base.powf(max_exponent as f64).ceil() < f64::from(std::u32::MAX),
            "Maximum interval multiplier should be less then u32::Max"
        );
        ExponentialTimeInterval {
            base_ms: base.as_millis() as u64, // any reasonable ms timeout fits u64 perfectly
            exponent_base,
            max_exponent,
        }
    }
}

impl PacemakerTimeInterval for ExponentialTimeInterval {
    fn get_round_duration(&self, round_index_after_committed_qc: usize) -> Duration {
        let pow = round_index_after_committed_qc.min(self.max_exponent) as u32;
        let base_multiplier = self.exponent_base.powf(f64::from(pow));
        let duration_ms = ((self.base_ms as f64) * base_multiplier).ceil() as u64;
        Duration::from_millis(duration_ms)
    }
}

/// `LocalPacemakerInner` is a Pacemaker implementation that relies on increasing local timeouts
/// in order to eventually come up with the timeout that is large enough to guarantee overlap of the
/// "current round" of multiple participants.
///
/// The protocol is as follows:
/// * `LocalPacemakerInner` manages the `highest_certified_round` that is keeping the round of the
/// highest certified block known to the validator.
/// * Once a new QC arrives with a round larger than that of `highest_certified_round`,
/// local pacemaker is going to increment a round with a default timeout.
/// * Upon every timeout `LocalPacemaker` increments a round and doubles the timeout.
///
/// `LocalPacemakerInner` does not require clock synchronization to maintain the property of
/// liveness - although clock synchronization can improve the time necessary to get a large enough
/// timeout overlap.
/// It does rely on an assumption that when an honest replica receives a quorum certificate
/// indicating to move to the next round, all other honest replicas will move to the next round
/// within a bounded time. This can be guaranteed via all honest replicas gossiping their highest
/// QC to f+1 other replicas for instance.
/// `LocalPacemakerInner`是一个Pacemaker实现，它依赖于增加本地超时，以便最终得出足够大的超时，
/// 以保证多个参与者的“当前轮次”的重叠。
///
/// 协议如下：
/// *`LocalPacemakerInner`管理highest_certified_round`，它保持验证者已知的最高认证块的轮次。
/// *一旦新QC到达的轮次大于“highest_certified_round”，本地起搏器将以默认超时增加一轮。
/// *每次超时“LocalPacemaker”增加一轮并使超时加倍。
///
/// `LocalPacemakerInner`不需要时钟同步来维持活跃性 - 尽管时钟同步可以改善获得足够大的超时重叠所需的时间。
/// 它确实依赖于这样的假设，即当一个诚实的副本收到一个表示转移到下一轮的法定证书时，所有其他诚实的副本
/// 将转移到下一轮
/// 在有限的时间内。这可以通过所有诚实的复制品保证最高的闲聊
/// 例如，QC到f + 1个其他副本
struct LocalPacemakerInner {
    // Determines the time interval for a round interval
    // 确定舍入间隔的时间间隔
    time_interval: Box<dyn PacemakerTimeInterval>,
    // Highest round that a block was committed
     // 块的最高回合
    highest_committed_round: Round,
    // Highest round known certified by QC.
    // 已通过质量控制认证的最高回合。
    highest_qc_round: Round,
    // Current round (current_round - highest_qc_round determines the timeout).
    // Current round is basically max(highest_qc_round, highest_received_tc, highest_local_tc) + 1
    // update_current_round take care of updating current_round and sending new round event if
    // it changes
    // 当前轮次（current_round  -  highest_qc_round确定超时）。
    //
    // 当前轮次基本上是最大值（highest_qc_round，highest_received_tc，highest_local_tc）+ 1
    //
    // update_current_round负责更新current_round并在发生更改时发送新的round事件
    current_round: Round,
    // Approximate deadline when current round ends
    // 当前轮次结束时的近似截止日期
    current_round_deadline: Instant,
    // Service for timer
     // 为计时器服务
    time_service: Arc<dyn TimeService>,
    // To send new round events.
    // 发送新一轮事件。
    new_round_events_sender: channel::Sender<NewRoundEvent>,
    // To send timeout events to itself.
    // 向自己发送超时事件。
    local_timeout_sender: channel::Sender<Round>,
    // To send timeout events to other pacemakers
    // 将超时事件发送给其他心脏起搏器
    external_timeout_sender: channel::Sender<Round>,
    // Manages the PacemakerTimeout and PacemakerTimeoutCertificate structs
    // 管理PacemakerTimeout和PacemakerTimeoutCertificate结构
    pacemaker_timeout_manager: PacemakerTimeoutManager,
}

impl LocalPacemakerInner {
    pub fn new(
        persistent_liveness_storage: Box<dyn PersistentLivenessStorage>,
        time_interval: Box<dyn PacemakerTimeInterval>,
        highest_committed_round: Round,
        highest_qc_round: Round,
        time_service: Arc<dyn TimeService>,
        new_round_events_sender: channel::Sender<NewRoundEvent>,
        local_timeout_sender: channel::Sender<Round>,
        external_timeout_sender: channel::Sender<Round>,
        pacemaker_timeout_quorum_size: usize,
        highest_timeout_certificates: HighestTimeoutCertificates,
    ) -> Self {
        assert!(pacemaker_timeout_quorum_size > 0);
        // The starting round is maximum(highest quorum certificate,
        // highest timeout certificate round) + 1.  Note that it is possible this
        // replica already voted at this round and will until a round timeout
        // or another replica convinces it via a quorum certificate or a timeout
        // certificate to advance to a higher round.
        // 起始回合是最大值（最高法定人数证书，最高超时证书回合）+ 1.请注意，
        // 此副本可能已经在此回合中投票，直到轮次超时或其他副本通过法定证书或超时证书来说服它 进入更高的一轮。
        let current_round = {
            match highest_timeout_certificates.highest_timeout_certificate() {
                Some(highest_timeout_certificate) => {
                    cmp::max(highest_qc_round, highest_timeout_certificate.round())
                }
                None => highest_qc_round,
            }
        } + 1;
        // Our counters are initialized via lazy_static, so they're not going to appear in
        // Prometheus if some conditions never happen. Invoking get() function enforces creation.
         // 我们的计数器是通过lazy_static初始化的，所以如果某些条件永远不会发生，它们就不会出现在
        // Prometheus中。 调用get（）函数可以强制创建。
        counters::QC_ROUNDS_COUNT.get();
        counters::TIMEOUT_ROUNDS_COUNT.get();
        counters::TIMEOUT_COUNT.get();

        Self {
            time_interval,
            highest_committed_round,
            highest_qc_round,
            current_round,
            current_round_deadline: Instant::now(),
            time_service,
            new_round_events_sender,
            local_timeout_sender,
            external_timeout_sender,
            pacemaker_timeout_manager: PacemakerTimeoutManager::new(
                pacemaker_timeout_quorum_size,
                highest_timeout_certificates,
                persistent_liveness_storage,
            ),
        }
    }

    /// Trigger an event to create a new round interval and ignore any events from previous round
    /// intervals.  The reason for the event is given by the caller, the timeout is
    /// deterministically determined by the reason and the internal state.
     /// 触发事件以创建新的轮次间隔，并忽略先前轮次间隔中的任何事件。 事件的原因由调用者给出，
    /// 超时由确定的原因和内部状态决定。
    fn create_new_round_task(&mut self, reason: NewRoundReason) -> impl Future<Output = ()> + Send {
        let round = self.current_round;
        let timeout = self.setup_timeout();
        let mut sender = self.new_round_events_sender.clone();
        async move {
            if let Err(e) = sender
                .send(NewRoundEvent {
                    round,
                    reason,
                    timeout,
                })
                .await
            {
                debug!("Error in sending new round interval event: {:?}", e);
            }
        }
    }

    /// Process local timeout for a given round: in case a new timeout message should be sent
    /// return a channel for sending the timeout message.
    /// 处理给定轮次的本地超时：如果应发送新的超时消息，则返回用于发送超时消息的通道。
    fn process_local_timeout(&mut self, round: Round) -> Option<channel::Sender<Round>> {
        if round != self.current_round {
            return None;
        }
        warn!(
            "Round {} has timed out, broadcasting new round message to all replicas",
            round
        );
        counters::TIMEOUT_COUNT.inc();
        self.setup_timeout();
        Some(self.external_timeout_sender.clone())
    }

    /// Setup the timeout task and return the duration of the current timeout
    /// 设置超时任务并返回当前超时的持续时间
    fn setup_timeout(&mut self) -> Duration {
        let timeout_sender = self.local_timeout_sender.clone();
        let timeout = self.setup_deadline();
        // Note that the timeout should not be driven sequentially with any other events as it can
        // become the head of the line blocker.
         // 请注意，不应该使用任何其他事件顺序驱动超时，因为它可能成为行阻止程序的头部。
        trace!(
            "Scheduling timeout of {} ms for round {}",
            timeout.as_millis(),
            self.current_round
        );
        self.time_service
            .run_after(timeout, SendTask::make(timeout_sender, self.current_round));
        timeout
    }

    /// Setup the current round deadline and return the duration of the current round
    /// 设置当前轮次截止日期并返回当前轮次的持续时间
    fn setup_deadline(&mut self) -> Duration {
        let round_index_after_committed_round = {
            if self.highest_committed_round == 0 {
                // Genesis doesn't require the 3-chain rule for commit, hence start the index at
                // the round after genesis.
                 // Genesis不需要3链规则进行提交，因此在创建后的回合中启动索引。
                self.current_round - 1
            } else {
                if self.current_round - self.highest_committed_round < 3 {
                    warn!("Finding a deadline for a round {} that should have already been completed since the highest committed round is {}",
                          self.current_round,
                          self.highest_committed_round);
                }

                max(0, self.current_round - self.highest_committed_round - 3)
            }
        } as usize;
        let timeout = self
            .time_interval
            .get_round_duration(round_index_after_committed_round);
        self.current_round_deadline = Instant::now() + timeout;
        timeout
    }

    /// Attempts to update highest_qc_certified_round when receiving QC for given round.
    /// Returns true if highest_qc_certified_round of this pacemaker has changed
       /// 尝试在接收给定轮次的QC时更新highest_qc_certified_round。
    /// 如果此起搏器的highest_qc_certified_round已更改，则返回true
    fn update_highest_qc_round(&mut self, round: Round) -> bool {
        if round > self.highest_qc_round {
            debug!(
                "{}QuorumCertified at {}{}",
                Fg(LightBlack),
                round,
                Fg(Reset)
            );
            self.highest_qc_round = round;
            return true;
        }
        false
    }

    /// Combines highest_qc_certified_round, highest_local_tc and highest_received_tc into
    /// effective round of this pacemaker.
    /// Generates new_round event if effective round changes and ensures it is
    /// monotonically increasing
 
    /// 将highest_qc_certified_round，highest_local_tc和highest_received_tc组合成此起搏器的有效轮次。
    /// 如果有效的轮次更改生成new_round事件并确保它单调增加
    fn update_current_round(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let (mut best_round, mut best_reason) = (self.highest_qc_round, NewRoundReason::QCReady);
        if let Some(highest_timeout_certificate) =
            self.pacemaker_timeout_manager.highest_timeout_certificate()
        {
            if highest_timeout_certificate.round() > best_round {
                best_round = highest_timeout_certificate.round();
                best_reason = NewRoundReason::Timeout {
                    cert: highest_timeout_certificate.clone(),
                };
            }
        }

        let new_round = best_round + 1;
        if self.current_round == new_round {
            debug!(
                "{}Round did not change: {}{}",
                Fg(LightBlack),
                new_round,
                Fg(Reset)
            );
            return async {}.boxed();
        }
        assert!(
            new_round > self.current_round,
            "Round illegally decreased from {} to {}",
            self.current_round,
            new_round
        );
        self.current_round = new_round;
        self.create_new_round_task(best_reason).boxed()
    }

    /// Validate timeout certificate and update local state if it's correct
    /// 验证超时证书并更新本地状态（如果正确）
    fn check_and_update_highest_received_tc(
        &mut self,
        tc: Option<&PacemakerTimeoutCertificate>,
    ) -> bool {
        if let Some(tc) = tc {
            return self
                .pacemaker_timeout_manager
                .update_highest_received_timeout_certificate(tc);
        }
        false
    }
}

/// `LocalPacemaker` is a wrapper to make the `LocalPacemakerInner` thread-safe.
/// `LocalPacemaker`是一个使`LocalPacemakerInner`线程安全的包装器。
pub struct LocalPacemaker {
    inner: Arc<RwLock<LocalPacemakerInner>>,
}

impl LocalPacemaker {
    pub fn new(
        executor: TaskExecutor,
        persistent_liveness_storage: Box<dyn PersistentLivenessStorage>,
        time_interval: Box<dyn PacemakerTimeInterval>,
        highest_committed_round: Round,
        highest_qc_round: Round,
        time_service: Arc<dyn TimeService>,
        new_round_events_sender: channel::Sender<NewRoundEvent>,
        external_timeout_sender: channel::Sender<Round>,
        pacemaker_timeout_quorum_size: usize,
        highest_timeout_certificates: HighestTimeoutCertificates,
    ) -> Self {
        let (local_timeouts_sender, mut local_timeouts_receiver) =
            channel::new(1_024, &counters::PENDING_PACEMAKER_TIMEOUTS);
        let inner = Arc::new(RwLock::new(LocalPacemakerInner::new(
            persistent_liveness_storage,
            time_interval,
            highest_committed_round,
            highest_qc_round,
            time_service,
            new_round_events_sender,
            local_timeouts_sender,
            external_timeout_sender,
            pacemaker_timeout_quorum_size,
            highest_timeout_certificates,
        )));

        let inner_ref = Arc::clone(&inner);
        let timeout_processing_loop = async move {
            // To jump start the execution return the new round event for the current round.
             // 要快速启动，执行将返回当前轮次的新轮次事件。
            inner_ref
                .write()
                .unwrap()
                .create_new_round_task(NewRoundReason::QCReady)
                .await;

            // Start the loop of processing local timeouts
            // 启动处理本地超时的循环
            while let Some(round) = local_timeouts_receiver.next().await {
                Self::process_local_timeout(Arc::clone(&inner_ref), round).await;
            }
        };
        executor.spawn(timeout_processing_loop.boxed().unit_error().compat());

        Self { inner }
    }

    async fn process_local_timeout(inner: Arc<RwLock<LocalPacemakerInner>>, round: Round) {
        let timeout_processing_res = { inner.write().unwrap().process_local_timeout(round) };
        if let Some(mut sender) = timeout_processing_res {
            if let Err(e) = sender.send(round).await {
                warn!("Can't send pacemaker timeout message: {:?}", e)
            }
        }
    }
}

impl Pacemaker for LocalPacemaker {
    fn current_round_deadline(&self) -> Instant {
        self.inner.read().unwrap().current_round_deadline
    }

    fn current_round(&self) -> Round {
        self.inner.read().unwrap().current_round
    }

    fn process_certificates(
        &self,
        qc_round: Round,
        timeout_certificate: Option<&PacemakerTimeoutCertificate>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let mut guard = self.inner.write().unwrap();
        let tc_round_updated = guard.check_and_update_highest_received_tc(timeout_certificate);
        let qc_round_updated = guard.update_highest_qc_round(qc_round);
        if tc_round_updated || qc_round_updated {
            return guard.update_current_round();
        }
        async {}.boxed()
    }

    /// The function is invoked upon receiving a remote timeout message from another validator.
    /// 在从另一个验证器接收到远程超时消息时调用该函数。
    fn process_remote_timeout(
        &self,
        pacemaker_timeout: PacemakerTimeout,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let mut guard = self.inner.write().unwrap();
        if guard
            .pacemaker_timeout_manager
            .update_received_timeout(pacemaker_timeout)
        {
            return guard.update_current_round();
        }
        async {}.boxed()
    }

    fn update_highest_committed_round(&self, highest_committed_round: Round) {
        let mut guard = self.inner.write().unwrap();
        if guard.highest_committed_round < highest_committed_round {
            guard.highest_committed_round = highest_committed_round;
        }
    }
}
