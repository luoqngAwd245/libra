// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::util::time_service::{ScheduledTask, TimeService};
use futures::{Future, FutureExt};
use logger::prelude::*;
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

/// SimulatedTimeService implements TimeService, however it does not depend on actual time
/// There are multiple ways to use it:
/// SimulatedTimeService::new will create time service that simply 'stuck' on time 0
/// SimulatedTimeService::update_auto_advance_limit can then be used to allow time to advance up to
/// certain limit. SimulatedTimeService::auto_advance_until will create time service that will 'run'
/// until certain time limit Note that SimulatedTimeService does not actually wait for any timeouts,
/// notion of time in it is abstract. Tasks run asap as long as they are scheduled before configured
/// time limit
/// SimulatedTimeService实现了TimeService，但它不依赖于实际时间有多种方法可以使用它：
/// SimulatedTimeService :: new将创建时间服务，只是在时间0上“卡住”
/// 然后可以使用SimulatedTimeService :: update_auto_advance_limit来允许时间推进到某个限制。 SimulatedTimeService ::
/// auto_advance_until将创建将“运行”直到特定时间限制的时间服务请注意，SimulatedTimeService实际上并不等待任何超时，
/// 其中的时间概念是抽象的。 任务只要在配置的时间限制之前安排就会尽快运行
pub struct SimulatedTimeService {
    inner: Arc<Mutex<SimulatedTimeServiceInner>>,
}

struct SimulatedTimeServiceInner {
    now: Duration,
    pending: Vec<(Duration, Box<dyn ScheduledTask>)>,
    time_limit: Duration,
    /// Maximum duration self.now is allowed to advance to
    /// 最大持续时间self.now允许前进到
    max: Duration,
}

impl TimeService for SimulatedTimeService {
    fn run_after(&self, timeout: Duration, mut t: Box<dyn ScheduledTask>) {
        let mut inner = self.inner.lock().unwrap();
        let now = inner.now;
        let deadline = now + timeout;
        if deadline > inner.time_limit {
            debug!(
                "sched for deadline: {}, now: {}, limit: {}",
                deadline.as_millis(),
                now.as_millis(),
                inner.time_limit.as_millis()
            );
            inner.pending.push((deadline, t));
        } else {
            debug!(
                "exec deadline: {}, now: {}",
                deadline.as_millis(),
                now.as_millis()
            );
            inner.now = deadline;
            if inner.now > inner.max {
                inner.now = inner.max;
            }
            // Perhaps this could be done better, but I think its good enough for tests...
	    // 也许这可以做得更好，但我认为它足以进行测试......
            futures::executor::block_on(t.run());
        }
    }

    fn get_current_timestamp(&self) -> Duration {
        self.inner.lock().unwrap().now
    }

    fn sleep(&self, t: Duration) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let inner = self.inner.clone();
        let fut = async move {
            let mut inner = inner.lock().unwrap();
            inner.now += t;
            if inner.now > inner.max {
                inner.now = inner.max;
            }
        };
        fut.boxed()
    }
}

impl SimulatedTimeService {
    /// Creates new SimulatedTimeService in disabled state (time not running)
    /// 在禁用状态下创建新的SimulatedTimeService（时间未运行）
    pub fn new() -> SimulatedTimeService {
        SimulatedTimeService {
            inner: Arc::new(Mutex::new(SimulatedTimeServiceInner {
                now: Duration::from_secs(0),
                pending: vec![],
                time_limit: Duration::from_secs(0),
                max: Duration::from_secs(std::u64::MAX),
            })),
        }
    }

    /// Creates new SimulatedTimeService in disabled state (time not running) with a max duration
    /// 以最长持续时间创建处于禁用状态（时间未运行）的新SimulatedTimeService
    pub fn max(max: Duration) -> SimulatedTimeService {
        SimulatedTimeService {
            inner: Arc::new(Mutex::new(SimulatedTimeServiceInner {
                now: Duration::from_secs(0),
                pending: vec![],
                time_limit: Duration::from_secs(0),
                max,
            })),
        }
    }

    /// Creates new SimulatedTimeService that automatically advance time up to time_limit
    /// 创建新的SimulatedTimeService，自动将时间提前到time_limit
    pub fn auto_advance_until(time_limit: Duration) -> SimulatedTimeService {
        SimulatedTimeService {
            inner: Arc::new(Mutex::new(SimulatedTimeServiceInner {
                now: Duration::from_secs(0),
                pending: vec![],
                time_limit,
                max: Duration::from_secs(std::u64::MAX),
            })),
        }
    }

    /// Update time_limit of this SimulatedTimeService instance and run pending tasks that has
    /// deadline lower then new time_limit
    /// 更新此SimulatedTimeService实例的time_limit并运行截止时间低于新time_limit的挂起任务
    #[allow(dead_code)]
    pub fn update_auto_advance_limit(&mut self, time: Duration) {
        let mut inner = self.inner.lock().unwrap();
        inner.time_limit += time;
        let time_limit = inner.time_limit;
        let mut i = 0;
        let mut drain = vec![];
        while i != inner.pending.len() {
            let deadline = inner.pending[i].0;
            if deadline <= time_limit {
                drain.push(inner.pending.remove(i));
            } else {
                i += 1;
            }
        }
        for (_, mut t) in drain {
            // probably could be done better then that, but for now I feel its good enough for tests
            futures::executor::block_on(t.run());
        }
    }
}

impl Clone for SimulatedTimeService {
    fn clone(&self) -> SimulatedTimeService {
        SimulatedTimeService {
            inner: self.inner.clone(),
        }
    }
}
