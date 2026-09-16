//! 会话真相源：append-only JSONL
//!
//! 设计来自 DSH 的 append-only 事件流，并升级为：
//! 同一份 JSONL 既是审计日志，也是回放测试的输入与期望输出。

use neo_protocol::{EventMsg, Op, Seq};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
    pub v: u32,
    pub ts: String,
    pub seq: Seq,
    pub kind: String,
    pub payload: serde_json::Value,
}

pub struct JsonlWriter {
    file: File,
    next_seq: Seq,
}

impl JsonlWriter {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let next_seq = Self::tail_seq(path)? + 1;
        Ok(Self { file, next_seq })
    }

    fn tail_seq(path: &Path) -> std::io::Result<Seq> {
        if !path.exists() { return Ok(0); }
        let f = File::open(path)?;
        let mut last = 0u64;
        for line in BufReader::new(f).lines() {
            let line = line?;
            if line.trim().is_empty() { continue; }
            if let Ok(e) = serde_json::from_str::<SessionEvent>(&line) { last = last.max(e.seq); }
        }
        Ok(last)
    }

    /// append-only：seq 由写入器严格递增分配，调用方无法指定
    pub fn append(&mut self, kind: &str, payload: serde_json::Value) -> std::io::Result<Seq> {
        let s = self.next_seq;
        self.next_seq += 1;
        let ev = SessionEvent { v: 1, ts: "1970-01-01T00:00:00Z".into(), seq: s, kind: kind.into(), payload };
        writeln!(self.file, "{}", serde_json::to_string(&ev)?)?;
        self.file.flush()?;
        Ok(s)
    }

    pub fn append_op(&mut self, op: &Op) -> std::io::Result<Seq> {
        let payload = serde_json::to_value(op).unwrap_or_default();
        self.append("op", payload)
    }

    pub fn append_event(&mut self, ev: &EventMsg) -> std::io::Result<Seq> {
        let payload = serde_json::to_value(ev).unwrap_or_default();
        self.append("event", payload)
    }
}

/// 从 JSONL 重建会话：严格校验 seq 递增且无空洞
pub fn replay(path: &Path) -> std::io::Result<Vec<SessionEvent>> {
    let f = File::open(path)?;
    let mut out = Vec::new();
    for line in BufReader::new(f).lines() {
        let line = line?;
        if line.trim().is_empty() { continue; }
        out.push(serde_json::from_str::<SessionEvent>(&line)?);
    }
    Ok(out)
}

pub fn assert_contiguous(events: &[SessionEvent]) -> Result<(), String> {
    for (i, e) in events.iter().enumerate() {
        let expected = (i + 1) as Seq;
        if e.seq != expected {
            return Err(format!("seq 不连续：位置 {} 期望 {} 实际 {}", i, expected, e.seq));
        }
    }
    Ok(())
}

/// 会话控制契据：宿主用它**列出 / 切换 / 新建 / 删除**会话。
///
/// # 为什么定义在共享层而不是某个宿主里
///
/// 它原本长在 `neo-host-tui` 里。当第二个宿主（桌面原生 GUI）也需要会话管理时，
/// 架构守卫 A3 **禁止宿主互相依赖**，共享契约必须沉到宿主之下 —— 于是搬到这里。
/// 这与 `neo-text` 的搬迁是同一个理由（见 PROJECT_MEMORY §4.63(e)）。
///
/// # 为什么是契据而不是让宿主直接依赖存储实现
///
/// 宿主**不持有会话存储**：`switch` 要同时做两件事 —— 让**内核**换会话并重建
/// 上下文、再把重建出的历史**转回事件流**给宿主重画转录。前者只有内核能做
/// （它持有 messages/session_id），后者是宿主的事。用契据把这对动作包成一个
/// 调用，宿主就不必知道"内核怎么换会话"，也不必碰 `SessionPersistence`。
///
/// 实现方通常是 CLI 装配层（`neo-code-cli` 的 `TuiSessions`：同时握着内核句柄
/// 与会话库）—— 它把两者接起来，宿主只看到这个契据。
///
/// # 返回事件流而不是"状态"
///
/// `switch` 返回 `Vec<EventMsg>` 而不是某个会话状态结构：宿主重画转录需要的是
/// **事件**（与实时流同一种形态），这样"重放历史"与"接收新事件"走同一条渲染
/// 路径，不必为历史单独写一套画法（那正是两份实现漂移的来源）。
pub trait SessionControl {
    /// 列出现有会话：(id, 标题, 记录数)。最近修改的在前。
    fn list(&self) -> Vec<(String, String, usize)>;

    /// 切换到指定会话；返回该会话的历史事件流。
    fn switch(&mut self, id: &str) -> Result<Vec<neo_protocol::EventMsg>, String>;

    /// 新建会话；返回新 id（旧会话保留在磁盘上，可再切回）。
    fn create(&mut self) -> Result<String, String>;

    /// 删除会话；`Ok(false)` 表示本来就不存在。
    fn delete(&mut self, id: &str) -> Result<bool, String>;

    /// 当前会话 id。
    fn current(&self) -> String;
}

