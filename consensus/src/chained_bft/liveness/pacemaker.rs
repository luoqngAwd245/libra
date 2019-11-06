// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::chained_bft::{
    common::Round,
    consensus_types::timeout_msg::{PacemakerTimeout, PacemakerTimeoutCertificate},
};
use futures::Future;
use std::{
    fmt,
    pin::Pin,
    time::{Duration, Instant},
};

/// A reason for starting a new round: introduced for monitoring / debug purposes.
/// 启动新一轮的原因：为监控/调试目的而引入。
#[derive(Eq, Debug, PartialEq)]
pub enum NewRoundReason {
    QCReady,
    Timeout { cert: PacemakerTimeoutCertificate },
}

impl fmt::Display for NewRoundReason {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            NewRoundReason::QCReady => write!(f, "QCReady"),
            NewRoundReason::Timeout { cert } => write!(f, "{}", cert),
        }
    }
}

/// NewRoundEvents produced by Pacemaker are guaranteed to be monotonically increasing.
/// NewRoundEvents are consumed by the rest of the system: they can cause sending new proposals
/// or voting for some proposals that wouldn't have been voted otherwise.
/// The duration is populated for debugging and testing
/// 由Pacemaker生产的NewRoundEvents保证单调增加。
/// NewRoundEvents被系统的其余部分消耗：它们可能导致发送新提案或投票给一些本来不会被投票的提案。
/// 填充持续时间以进行调试和测试
#[derive(Debug, PartialEq, Eq)]
pub struct NewRoundEvent {
    pub round: Round,
    pub reason: NewRoundReason,
    pub timeout: Duration,
}

impl fmt::Display for NewRoundEvent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "NewRoundEvent: [round: {}, reason: {}, timeout: {:?}]",
            self.round, self.reason, self.timeout
        )
    }
}

/// Pacemaker is responsible for generating the new round events, which are driving the actions
/// of the rest of the system (e.g., for generating new proposals).
/// Ideal pacemaker provides an abstraction of a "shared clock". In reality pacemaker
/// implementations use external signals like receiving new votes / QCs plus internal
/// communication between other nodes' pacemaker instances in order to synchronize the logical
/// clocks.
/// The trait doesn't specify the starting conditions or the executor that is responsible for
/// driving the logic.
/// 起搏器负责产生新的轮次事件，这些事件正在推动系统其余部分的动作（例如，用于产生新的提议）。
/// 理想的起搏器提供了“共享时钟”的抽象。 实际上，起搏器实施使用外部信号，例如接收新的投票/ QC以及
/// 其他节点的起搏器实例之间的内部通信，以便同步逻辑时钟。
/// 特征没有指定起始条件或负责驱动逻辑的执行者。
pub trait Pacemaker: Send + Sync {
    /// Returns deadline for current round
    /// 返回当前回合的截止日期
    fn current_round_deadline(&self) -> Instant;

    /// Synchronous function to return the current round.
    /// 同步功能返回当前回合。
    fn current_round(&self) -> Round;

    /// Function to update current round based on received certificates.
    /// Both round of latest received QC and timeout certificates are taken into account.
    /// This function guarantees to update pacemaker state when promise that it returns is fulfilled
    /// 根据收到的证书更新当前轮次的功能。
    /// 两轮最新收到的QC和超时证书都被考虑在内。
    /// 此功能可确保在承诺返回时更新起搏器状态
    fn process_certificates(
        &self,
        qc_round: Round,
        timeout_certificate: Option<&PacemakerTimeoutCertificate>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>>;

    /// The function is invoked upon receiving a remote timeout message from another validator.
    /// 在从另一个验证器接收到远程超时消息时调用该函数。
    fn process_remote_timeout(
        &self,
        pacemaker_timeout: PacemakerTimeout,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>>;

    /// Update the highest committed round
    /// 更新最高承诺回合
    fn update_highest_committed_round(&self, highest_committed_round: Round);
}
