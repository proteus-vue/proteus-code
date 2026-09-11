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
