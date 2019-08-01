// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use channel;
use futures::{compat::Future01CompatExt, Future, FutureExt, SinkExt, TryFutureExt};
use logger::prelude::*;
use std::{
    pin::Pin,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{executor::Executor, runtime::TaskExecutor, timer::Delay};

/// Time service is an abstraction for operations that depend on time
/// It supports implementations that can simulated time or depend on actual time
/// We can use simulated time in tests so tests can run faster and be more stable.
/// see SimulatedTime for implementation that tests should use
/// Time service also supports opportunities for future optimizations
/// For example instead of scheduling O(N) tasks in TaskExecutor we could have more optimal code
/// that only keeps single task in TaskExecutor
/// 时间服务是依赖于时间的操作的抽象
///  它支持可以模拟时间或取决于实际时间的实现
/// 我们可以在测试中使用模拟时间，因此测试可以更快地运行并且更稳定。
/// 请参阅SimulatedTime以了解测试应该使用的实现
/// 时间服务还支持未来优化的机会
/// 例如，我们可以拥有更优化的代码，而不是在TaskExecutor中调度O（N）任务，而只在TaskExecutor中保留单个任务
pub trait TimeService: Send + Sync {
    /// Sends message to given sender after timeout
    /// 超时后向给定发件人发送消息
    fn run_after(&self, timeout: Duration, task: Box<dyn ScheduledTask>);

    /// Retrieve the current time stamp as a Duration (assuming it is on or after the UNIX_EPOCH)
    /// 将当前时间戳检索为Duration（假设它在UNIX_EPOCH之上或之后）
    fn get_current_timestamp(&self) -> Duration;

    /// Makes a future that will sleep for given Duration
    /// This function guarantees that get_current_timestamp will increase at least by
    /// given duration, e.g.
    /// X = time_service::get_current_timestamp();
    /// time_service::sleep(Y).await;
    /// Z = time_service::get_current_timestamp();
    /// assert(Z >= X + Y)
     /// 创造一个将在给定持续时间内睡觉的未来
    /// 该函数保证get_current_timestamp将至少增加给定的持续时间，例如
    /// X = time_service::get_current_timestamp();
    /// time_service::sleep(Y).await;
    /// Z = time_service::get_current_timestamp();
    /// assert(Z >= X + Y)
    fn sleep(&self, t: Duration) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

/// This trait represents abstract task that can be submitted to TimeService::run_after
/// 此特征表示可以提交到TimeService :: run_after的抽象任务
pub trait ScheduledTask: Send {
    /// TimeService::run_after will run this method when time expires
    /// It is expected that this function is lightweight and does not take long time to complete
    /// TimeService :: run_after将在时间到期时运行此方法
    /// 预计该功能重量轻，不需要很长时间才能完成
    fn run(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}

/// This tasks send message to given Sender
/// 此任务将消息发送给给定的发件人
pub struct SendTask<T>
where
    T: Send + 'static,
{
    sender: Option<channel::Sender<T>>,
    message: Option<T>,
}

impl<T> SendTask<T>
where
    T: Send + 'static,
{
    /// Makes new SendTask for given sender and message and wraps it to Box
    /// 为给定的发件人和消息创建新的SendTask并将其包装到Box
    pub fn make(sender: channel::Sender<T>, message: T) -> Box<dyn ScheduledTask> {
        Box::new(SendTask {
            sender: Some(sender),
            message: Some(message),
        })
    }
}

impl<T> ScheduledTask for SendTask<T>
where
    T: Send + 'static,
{
    fn run(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let mut sender = self.sender.take().unwrap();
        let message = self.message.take().unwrap();
        let r = async move {
            if let Err(e) = sender.send(message).await {
                error!("Error on send: {:?}", e);
            };
        };
        r.boxed()
    }
}

/// TimeService implementation that uses actual clock to schedule tasks
/// TimeService实现，使用实际时钟来计划任务
pub struct ClockTimeService {
    executor: TaskExecutor,
}

impl ClockTimeService {
    /// Creates new TimeService that runs tasks based on actual clock
    /// It needs executor to schedule internal tasks that facilitates it's work
    /// 创建基于实际时钟运行任务的新TimeService
    /// 它需要执行程序来安排促进其工作的内部任务
    pub fn new(executor: TaskExecutor) -> ClockTimeService {
        ClockTimeService { executor }
    }
}

impl TimeService for ClockTimeService {
    fn run_after(&self, timeout: Duration, mut t: Box<dyn ScheduledTask>) {
        let task = async move {
            let timeout_time = Instant::now() + timeout;
            if let Err(e) = Delay::new(timeout_time).compat().await {
                error!("Error on delay: {:?}", e);
            };
            t.run().await;
        };
        let task = task.boxed().unit_error().compat();
        let mut executor = self.executor.clone();
        if let Err(e) = Executor::spawn(&mut executor, Box::new(task)) {
            warn!("Failed to submit task to runtime: {:?}", e)
        }
    }

    fn get_current_timestamp(&self) -> Duration {
        duration_since_epoch()
    }

    fn sleep(&self, t: Duration) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        async move { Delay::new(Instant::now() + t).compat().await.unwrap() }.boxed()
    }
}

/// Return the duration since the UNIX_EPOCH
/// 返回自UNIX_EPOCH以来的持续时间
pub fn duration_since_epoch() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Timestamp generated is before the UNIX_EPOCH!")
}

/// Success states for wait_if_possible
#[derive(Debug, PartialEq, Eq)]
pub enum WaitingSuccess {
    /// No waiting to complete and includes the current duration since epoch and the difference
    /// between the current duration since epoch and min_duration_since_epoch
    /// 无需等待完成并包括自纪元以来的当前持续时间以及自纪元以来的当前持续时间与
    /// min_duration_since_epoch之间的差异
    NoWaitRequired {
        current_duration_since_epoch: Duration,
        early_duration: Duration,
    },
    /// Waiting was required and includes the current duration since epoch and the duration
    /// slept to finish waiting
    /// 等待是必需的，包括自纪元以来的当前持续时间以及等待完成等待的持续时间
    WaitWasRequired {
        current_duration_since_epoch: Duration,
        wait_duration: Duration,
    },
}

/// Error states for wait_if_possible
#[derive(Debug, PartialEq, Eq)]
pub enum WaitingError {
    /// The waiting period exceeds the maximum allowed duration, returning immediately
    /// 等待时间超过允许的最大持续时间，立即返回
    MaxWaitExceeded,
    /// Waiting to ensure the current time exceeds min_duration_since_epoch failed
     /// 等待确保当前时间超过min_duration_since_epoch失败
    WaitFailed {
        current_duration_since_epoch: Duration,
        wait_duration: Duration,
    },
}

/// Attempt to wait until the current time exceeds the min_duration_since_epoch if possible
///
/// If the waiting time exceeds max_instant then fail immediately.
/// There are 4 potential outcomes, 2 successful and 2 errors, each represented by
/// WaitingSuccess and WaitingError.
/// 如果可能，尝试等到当前时间超过min_duration_since_epoch
///
/// 如果等待时间超过max_instant，则立即失败。
/// 有4个潜在结果，2个成功结果和2个错误，每个结果由WaitingSuccess和WaitingError表示。
pub async fn wait_if_possible(
    time_service: &dyn TimeService,
    min_duration_since_epoch: Duration,
    max_instant: Instant,
) -> Result<WaitingSuccess, WaitingError> {
    // Fail early if waiting for min_duration_since_epoch would exceed max_instant
    // Ideally, comparing min_duration_since_epoch and max_instant would be straightforward, but
    // min_duration_since_epoch is relative to UNIX_EPOCH and Instant is not comparable.  Therefore,
    // we use relative differences to do the comparison.
    // 如果等待min_duration_since_epoch超过max_instant，则提前失败
    // 理想情况下，比较min_duration_since_epoch和max_instant会很简单，但min_duration_since_epoch相对于UNIX_EPOCH
    // 而Instant则无法比较。 因此，我们使用相对差异进行比较。
    let current_instant = Instant::now();
    let current_duration_since_epoch = time_service.get_current_timestamp();
    if current_instant <= max_instant {
        let duration_to_max_time = max_instant.duration_since(current_instant);
        if current_duration_since_epoch <= min_duration_since_epoch {
            let duration_to_min_time = min_duration_since_epoch - current_duration_since_epoch;
            if duration_to_max_time < duration_to_min_time {
                return Err(WaitingError::MaxWaitExceeded);
            }
        }
    }

    if current_duration_since_epoch <= min_duration_since_epoch {
        // Delay has millisecond granularity, add 1 millisecond to ensure a higher timestamp
        // 延迟具有毫秒级的粒度，增加1毫秒以确保更高的时间戳
        let sleep_duration =
            min_duration_since_epoch - current_duration_since_epoch + Duration::from_millis(1);
        time_service.sleep(sleep_duration).await;
        let waited_duration_since_epoch = time_service.get_current_timestamp();
        if waited_duration_since_epoch > min_duration_since_epoch {
            Ok(WaitingSuccess::WaitWasRequired {
                current_duration_since_epoch: waited_duration_since_epoch,
                wait_duration: sleep_duration,
            })
        } else {
            Err(WaitingError::WaitFailed {
                current_duration_since_epoch: waited_duration_since_epoch,
                wait_duration: sleep_duration,
            })
        }
    } else {
        Ok(WaitingSuccess::NoWaitRequired {
            current_duration_since_epoch,
            early_duration: current_duration_since_epoch - min_duration_since_epoch,
        })
    }
}
