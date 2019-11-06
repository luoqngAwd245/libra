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
//! �ṩ��װ��[IntGauge`]��ָ�꣺: IntGauge���е�mpsc���������ߵ������ߣ�ͨ����
//! ԭʼ��future mpscͨ������ÿ����¡�ķ����˶��б�֤�Ĳ�۵���Ϊ�������ǵĴ�����У���Щ�������Ҫ����
//! �����˲��������硰 buffer_unordered��֮��������һ��ʹ�á��н�mpscͨ���������ַ�ʽ��Ϊ�޽硣�ڴ�[PR]
//! ���кܶ����ۣ�https://github.com/rust-lang-nursery/futures-rs/pull/984������ǰ��Ϊ���۾���ÿ��������
//! �����б������ƣ�����������ȫ��Э������¡�����˵����������ǣ���ĳЩ�������ʵ�����ǲ����еġ�����
//! �������һ�ֽ����������poll_flush����poll_ready��������noop����ȷ����ǰ����������û�б�פ����
//! �����¿�¡�ķ����˳��Խ���Ϣ���͵�����ͨ���������sendִ��poll_ready��start_send��poll_flush��
//! ��һ��poll_ready�����ء�����������Ϊmaybe_parked��ʼ��Ϊfalse��Ȼ��start_send����Ϣ���͵��ڲ�
//! ��Ϣ���У���פ������������ poll_flush�ٴε���poll_ready����һ�Σ����ڷ�����������פ������������
//! Pending����ˣ����ͽ�һֱ������ֱ�������ߴӶ�����ɾ��������Ϣ�����Ҹ÷����ߵ�����δפ��Ϊֹ��
//! [�˹���]��https://github.com/rust-lang-nursery/futures-rs/pull/1671��Ӧ�����ڻ�0.3���޸������⡣
//! �ϲ��󽫱���һ�¡�
//!
//! ���ǣ��˸���ȷʵ����ĳЩ���塣
//! 1.ͨ����СΪ0ʱ����Ϊͬ���� �ڴӽ���������������Ʒ֮ǰ�������͡�������ɡ�
//! 2.������������յ���Ʒ����䣬�򡰷��͡����ܻ�ʧ�ܡ�
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
//! ���������ʾ����`tx.send`���ܻ�ʧ�ܡ� ��Ϊsend������������-poll_ready��start_send��poll_flush��
//! ��start_send֮��rx���Խ��ո���Ŀ���������rx��poll_flush֮ǰ����������ᴥ���Ͽ����ӵķ��ʹ���
//! ����ǽ��Ͽ����ӵĴ�����poll_flush��ת��ΪOk��ԭ��

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
/// ��װ����IntGauge��ֵ
/// �����ں���mpsc :: channel��Ԫ�ص�����
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
