// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! Provides an mpsc (multi-producer single-consumer) channel wrapped in an
//! [`IntGauge`](metrics::IntGauge)
//!
//! The original futures mpsc channels has the behavior that each cloned sender gets a guaranteed
//! slot. There are cases in our codebase that senders need to be cloned to work with combinators
//! like `buffer_unordered`. The bounded mpsc channels turn to be unbounded in this way.  There are
//! great discussions in this [PR](https://github.com/rust-lang-nursery/futures-rs/pull/984). The
//! argument of the current behavior is to have only local limit on each sender, and relies on
//! global coordination for the number of cloned senders.  However, this isn't really feasible in
//! some cases. One solution that came up from the discussion is to have poll_flush call poll_ready
//! (instead of a noop) to make sure the current sender task isn't parked.  For the case that a new
//! cloned sender tries to send a message to a full channel, send executes poll_ready, start_send
//! and poll_flush. The first poll_ready would return Ready because maybe_parked initiated as
//! false. start_send then pushes the message to the internal message queue and parks the sender
//! task.  poll_flush calls poll_ready again, and this time, it would return Pending because the
//! sender task is parked. So the send will block until the receiver removes more messages from the
//! queue and that sender's task is unparked.
//! [This PR](https://github.com/rust-lang-nursery/futures-rs/pull/1671) is supposed to fix this in
//! futures 0.3. It'll be consistent once it's merged.
//!
//! This change does have some implications though.
//! 1. When the channel size is 0, it becomes synchronous. `send` won't finish until the item is
//! taken from the receiver.
//! 2. `send` may fail if the receiver drops after receiving the item.
//!
//! let (tx, rx) = channel::new_test(1);
//! let f_tx = async move {
//!     block_on(tx.send(1)).unwrap();
//! };
//! let f_rx = async move {
//!     let item = block_on(rx.next()).unwrap();
//!     assert_eq!(item, 1);
//! };
//! block_on(join(f_tx, f_rx)).unwrap();
//!
//! For the example above, `tx.send` could fail. Because send has three steps - poll_ready,
//! start_send and poll_flush. After start_send, the rx can receive the item, but if rx gets
//! dropped before poll_flush, it'll trigger disconnected send error. That's why the disconnected
//! error is converted to an Ok in poll_flush.
//!
//! 提供包装在[IntGauge`]（指标：: IntGauge）中的mpsc（多生产者单消费者）通道。
//! 原始的future mpsc通道具有每个克隆的发件人都有保证的插槽的行为。在我们的代码库中，有些情况下需要复制
//! 发件人才能与诸如“ buffer_unordered”之类的组合器一起使用。有界mpsc通道将以这种方式变为无界。在此[PR]
//! 中有很多讨论（https://github.com/rust-lang-nursery/futures-rs/pull/984）。当前行为的论据是每个发件人
//! 仅具有本地限制，并且依赖于全局协调来克隆发件人的数量。但是，在某些情况下这实际上是不可行的。讨论
//! 中提出的一种解决方案是让poll_flush调用poll_ready（而不是noop）来确保当前发件人任务没有被驻留。
//! 对于新克隆的发件人尝试将消息发送到完整通道的情况，send执行poll_ready，start_send和poll_flush。
//! 第一个poll_ready将返回“就绪”，因为maybe_parked初始化为false。然后，start_send将消息推送到内部
//! 消息队列，并驻留发送者任务。 poll_flush再次调用poll_ready，这一次，由于发送者任务已驻留，它将返回
//! Pending。因此，发送将一直阻塞，直到接收者从队列中删除更多消息，并且该发送者的任务未驻留为止。
//! [此公关]（https://github.com/rust-lang-nursery/futures-rs/pull/1671）应该在期货0.3中修复此问题。
//! 合并后将保持一致。
//!
//! 但是，此更改确实具有某些含义。
//! 1.通道大小为0时，变为同步。 在从接收者那里拿走物品之前，“发送”不会完成。
//! 2.如果接收者在收到物品后掉落，则“发送”可能会失败。
//!
//! let (tx, rx) = channel::new_test(1);
//! let f_tx = async move {
//!     block_on(tx.send(1)).unwrap();
//! };
//! let f_rx = async move {
//!     let item = block_on(rx.next()).unwrap();
//!     assert_eq!(item, 1);
//! };
//! block_on(join(f_tx, f_rx)).unwrap();
//!
//! 对于上面的示例，`tx.send`可能会失败。 因为send具有三个步骤-poll_ready，start_send和poll_flush。
//! 在start_send之后，rx可以接收该项目，但是如果rx在poll_flush之前被丢弃，则会触发断开连接的发送错误。
//! 这就是将断开连接的错误在poll_flush中转换为Ok的原因。

use futures::{
    channel::mpsc,
    sink::Sink,
    stream::{FusedStream, Stream},
    task::{Context, Poll},
};
use metrics::IntGauge;
use std::pin::Pin;

#[cfg(test)]
mod test;

/// Wrapper around a value with an `IntGauge`
/// It is used to gauge the number of elements in a `mpsc::channel`
/// 包装带有IntGauge的值
/// 它用于衡量mpsc :: channel中元素的数量
#[derive(Clone)]
pub struct WithGauge<T> {
    gauge: IntGauge,
    value: T,
}

/// Similar to `mpsc::Sender`, but with an `IntGauge`
pub type Sender<T> = WithGauge<mpsc::Sender<T>>;
/// Similar to `mpsc::Receiver`, but with an `IntGauge`
pub type Receiver<T> = WithGauge<mpsc::Receiver<T>>;

/// `Sender` implements `Sink` in the same way as `mpsc::Sender`, but it increments the
/// associated `IntGauge` when it sends a message successfully.
impl<T> Sink<T> for Sender<T> {
    type Error = mpsc::SendError;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        (*self).value.poll_ready(cx)
    }

    fn start_send(mut self: Pin<&mut Self>, msg: T) -> Result<(), Self::Error> {
        self.gauge.inc();
        (*self).value.start_send(msg).map_err(|e| {
            self.gauge.dec();
            e
        })?;
        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.value).poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.value).poll_close(cx)
    }
}

impl<T> Sender<T> {
    pub fn try_send(&mut self, msg: T) -> Result<(), mpsc::SendError> {
        self.gauge.inc();
        (*self).value.try_send(msg).map_err(|e| {
            self.gauge.dec();
            e.into_send_error()
        })
    }
}

impl<T> FusedStream for Receiver<T> {
    fn is_terminated(&self) -> bool {
        self.value.is_terminated()
    }
}

/// `Receiver` implements `Stream` in the same way as `mpsc::Stream`, but it decrements the
/// associated `IntGauge` when it gets polled successfully.
impl<T> Stream for Receiver<T> {
    type Item = T;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let poll = Pin::new(&mut self.value).poll_next(cx);
        if let Poll::Ready(Some(_)) = poll {
            self.gauge.dec();
        }
        poll
    }
}

/// Similar to `mpsc::channel`, `new` creates a pair of `Sender` and `Receiver`
pub fn new<T>(size: usize, gauge: &IntGauge) -> (Sender<T>, Receiver<T>) {
    gauge.set(0);
    let (sender, receiver) = mpsc::channel(size);
    (
        WithGauge {
            gauge: gauge.clone(),
            value: sender,
        },
        WithGauge {
            gauge: gauge.clone(),
            value: receiver,
        },
    )
}

lazy_static::lazy_static! {
    pub static ref TEST_COUNTER: IntGauge =
        IntGauge::new("TEST_COUNTER", "Counter of network tests").unwrap();
}

pub fn new_test<T>(size: usize) -> (Sender<T>, Receiver<T>) {
    new(size, &TEST_COUNTER)
}
