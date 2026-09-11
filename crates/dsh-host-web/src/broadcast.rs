//! 有界广播：把事件分发给多个订阅者（SSE 连接）。
//!
//! # 为什么必须有界
//!
//! 天真的实现是给每个订阅者一个无界队列。但只要有一个慢客户端
//! （浏览器标签页挂起、网络卡住），队列就会**无限增长**直到 OOM。
//! Rust 保内存安全，不保内存有界 —— 这是本模块存在的全部理由。
//!
//! 策略：队列有容量上限；**满了就丢弃该订阅者并断开**，而不是阻塞发布者
//! 或无限缓冲。丢一个慢客户端是局部损失；拖垮进程是全局损失。
//!
//! 这也符合 SSE 的语义：慢消费者本就该重连，而不是让服务端替它背历史。

use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};

/// 单个订阅者的队列容量。够吸收正常抖动，又不会让慢客户端拖垮内存。
pub const SUBSCRIBER_QUEUE: usize = 256;

/// 一个订阅者句柄。
pub struct Subscriber {
    id: u64,
    rx: Receiver<String>,
}

impl Subscriber {
    /// 阻塞取下一个消息；返回 None 表示发布端已关闭。
    pub fn recv(&self) -> Option<String> { self.rx.recv().ok() }
    pub fn id(&self) -> u64 { self.id }
}

struct Inner {
    senders: Vec<(u64, SyncSender<String>)>,
    next_id: u64,
}

/// 有界广播中心。
#[derive(Clone)]
pub struct Broadcast {
    inner: Arc<Mutex<Inner>>,
}

impl Broadcast {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(Inner { senders: Vec::new(), next_id: 1 })) }
    }

    /// 订阅。返回的句柄会在队列满时被自动摘除（后续 recv 返回 None）。
    pub fn subscribe(&self) -> Subscriber {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let id = inner.next_id;
        inner.next_id += 1;
        let (tx, rx) = sync_channel(SUBSCRIBER_QUEUE);
        inner.senders.push((id, tx));
        Subscriber { id, rx }
    }

    /// 主动退订（连接结束时调用，避免句柄泄漏）。
    pub fn unsubscribe(&self, id: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.senders.retain(|(sid, _)| *sid != id);
    }

    /// 发布一条消息。
    ///
    /// 队列满 → **摘除该订阅者**（丢慢客户端，不无限缓冲、不阻塞发布者）。
    /// 返回被摘除的订阅者数量，便于调用方记录"因过慢断开 N 个连接"。
    pub fn publish(&self, msg: &str) -> usize {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut dropped = 0;
        let mut alive = Vec::with_capacity(inner.senders.len());
        for (id, tx) in inner.senders.drain(..) {
            match tx.try_send(msg.to_string()) {
                Ok(()) => alive.push((id, tx)),
                Err(TrySendError::Full(_)) => {
                    // 慢客户端：断开。它的 SSE 连接会收到 None 并关闭。
                    dropped += 1;
                    // 刻意不保留：留一个塞满的队列毫无意义
                }
                Err(TrySendError::Disconnected(_)) => {
                    // 客户端已自行断开
                }
            }
        }
        inner.senders = alive;
        dropped
    }

    /// 当前订阅者数（诊断用）。
    pub fn subscriber_count(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).senders.len()
    }
}

impl Default for Broadcast {
    fn default() -> Self { Self::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivers_to_all_subscribers() {
        let b = Broadcast::new();
        let s1 = b.subscribe();
        let s2 = b.subscribe();
        assert_eq!(b.publish("hello"), 0);
        assert_eq!(s1.recv(), Some("hello".to_string()));
        assert_eq!(s2.recv(), Some("hello".to_string()));
    }

    #[test]
    fn drops_a_slow_subscriber_instead_of_buffering_forever() {
        let b = Broadcast::new();
        let s = b.subscribe();
        // 灌满队列（不消费）
        for i in 0..SUBSCRIBER_QUEUE {
            assert_eq!(b.publish(&format!("m{i}")), 0, "队列未满时不应丢弃");
        }
        // 再发一条 → 该订阅者被摘除
        assert_eq!(b.publish("overflow"), 1, "队列满时必须丢弃慢订阅者");
        assert_eq!(b.subscriber_count(), 0, "被丢弃的订阅者应被摘除");
        // 其接收端随之关闭（recv 在耗尽缓冲后返回 None）
        let mut count = 0;
        while s.recv().is_some() {
            count += 1;
            if count > SUBSCRIBER_QUEUE + 10 {
                panic!("不应无限可读");
            }
        }
    }

    #[test]
    fn unsubscribe_removes_the_handle() {
        let b = Broadcast::new();
        let s = b.subscribe();
        assert_eq!(b.subscriber_count(), 1);
        b.unsubscribe(s.id());
        assert_eq!(b.subscriber_count(), 0);
        assert_eq!(b.publish("x"), 0, "已退订者不算丢弃");
    }

    #[test]
    fn survivors_keep_receiving_after_a_drop() {
        let b = Broadcast::new();
        let slow = b.subscribe();
        let fast = b.subscribe();
        for i in 0..SUBSCRIBER_QUEUE {
            b.publish(&format!("m{i}"));
        }
        // 排空 fast，保持健康
        for i in 0..SUBSCRIBER_QUEUE {
            assert_eq!(fast.recv(), Some(format!("m{i}")));
        }
        // slow 已满 → 被丢弃；fast 仍在
        assert_eq!(b.publish("after"), 1, "只有 slow 应被丢弃");
        assert_eq!(fast.recv(), Some("after".to_string()), "健康订阅者不受影响");
        drop(slow);
    }
}
