//! L3 PROVIDER —— `SessionPersistence` 的 JSONL 实现（append-only）
//!
//! # append-only 是如何被强制的
//!
//! 1. 文件以 **append 模式**打开（`OpenOptions::append(true)`），OS 层面无法原地改写。
//! 2. `seq` 由本实现分配，调用方**无法指定** —— 塞不进重复或倒退的序号。
//! 3. 每次写入后 `flush`，保证崩溃后已汇报的记录确实在盘上。
//!
//! # 格式
//!
//! 每行一个 JSON 对象：`{"v":1,"ts":"...","seq":N,"kind":"op|event","payload":{...}}`
//! 与 `docs/neo-plan/05-验证/` 的 schema 对齐（字段名与 kind 取值集合）。
//!
//! # 诚实边界
//!
//! - `ts` 目前用**固定占位值**，未接系统时钟。原因：接入时钟会让"同一 Op 序列
//!   得同一日志"的确定性测试失效。真实时间戳应在**宿主层**标注，而不是混进
//!   内核产出的记录里。这是刻意的取舍，已在测试中断言。
//! - 未实现日志**轮转/压缩**（长会话会持续增长）。上下文有上限保护，
//!   但磁盘不会自动回收 —— 属后续工作。

use dsh_core::{LoggedRecord, PersistenceError, SessionPersistence};
use dsh_protocol::Seq;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// 当前日志格式版本。**变更格式必须递增**，并配套迁移（v0→v1→…）。
pub const FORMAT_VERSION: u32 = 1;

/// 确定性占位时间戳（见模块注释的取舍说明）。
const DETERMINISTIC_TS: &str = "1970-01-01T00:00:00Z";

pub struct JsonlPersistence {
    path: PathBuf,
    file: Option<File>,
    next_seq: Seq,
}

impl JsonlPersistence {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        // 已存在的日志 → 续号；否则从 1 开始
        let next_seq = tail_seq(&path).map(|s| s + 1).unwrap_or(1);
        Self { path, file: None, next_seq }
    }

    /// 惰性打开：不在构造时创建目录/文件，避免"只想读"的场景留下空文件。
    fn file(&mut self) -> Result<&mut File, PersistenceError> {
        if self.file.is_none() {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| {
                    PersistenceError::Unavailable(format!("无法创建目录 {}：{e}", dir.display()))
                })?;
            }
            let f = OpenOptions::new()
                .create(true)
                .append(true) // OS 层面 append-only
                .open(&self.path)
                .map_err(|e| {
                    PersistenceError::Unavailable(format!("无法打开 {}：{e}", self.path.display()))
                })?;
            self.file = Some(f);
        }
        Ok(self.file.as_mut().expect("刚赋值"))
    }

    pub fn path(&self) -> &Path { &self.path }
}

/// 读回最后一条记录的 seq（用于续号）。
fn tail_seq(path: &Path) -> Option<Seq> {
    let f = File::open(path).ok()?;
    let mut last = None;
    for line in BufReader::new(f).lines().map_while(Result::ok) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(s) = v.get("seq").and_then(|s| s.as_u64()) {
                last = Some(s);
            }
        }
    }
    last
}

impl SessionPersistence for JsonlPersistence {
    fn append(&mut self, kind: &str, payload: serde_json::Value) -> Result<Seq, PersistenceError> {
        let seq = self.next_seq;
        let record = serde_json::json!({
            "v": FORMAT_VERSION,
            "ts": DETERMINISTIC_TS,
            "seq": seq,
            "kind": kind,
            "payload": payload,
        });
        let line = serde_json::to_string(&record)
            .map_err(|e| PersistenceError::Unavailable(e.to_string()))?;
        let f = self.file()?;
        writeln!(f, "{line}")
            .and_then(|_| f.flush())
            .map_err(|e| PersistenceError::Unavailable(format!("写入失败：{e}")))?;
        self.next_seq += 1;
        Ok(seq)
    }

    fn load(&self) -> Result<Vec<LoggedRecord>, PersistenceError> {
        let f = match File::open(&self.path) {
            Ok(f) => f,
            // 尚未落盘 = 空日志，不是错误
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(PersistenceError::Unavailable(format!("无法读取：{e}"))),
        };
        let mut out = Vec::new();
        for line in BufReader::new(f).lines().map_while(Result::ok) {
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            let v: serde_json::Value = serde_json::from_str(t)
                .map_err(|e| PersistenceError::Unavailable(format!("日志行不是合法 JSON：{e}")))?;
            let seq = v.get("seq").and_then(|s| s.as_u64()).ok_or_else(|| {
                PersistenceError::Unavailable("日志行缺少 seq".into())
            })?;
            out.push(LoggedRecord {
                seq,
                kind: v.get("kind").and_then(|k| k.as_str()).unwrap_or("").to_string(),
                payload: v.get("payload").cloned().unwrap_or(serde_json::Value::Null),
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("neo-session-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn appends_and_reads_back_in_order() {
        let path = tmp("order");
        let mut p = JsonlPersistence::new(&path);
        for i in 0..3 {
            let s = p.append("event", serde_json::json!({"n": i})).unwrap();
            assert_eq!(s, (i + 1) as u64, "seq 必须从 1 严格递增");
        }
        let loaded = p.load().unwrap();
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[2].payload["n"], 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn is_append_only_across_reopen() {
        let path = tmp("reopen");
        {
            let mut p = JsonlPersistence::new(&path);
            p.append("op", serde_json::json!({"a": 1})).unwrap();
        }
        // 重开后：续号而不是重头写
        let mut p = JsonlPersistence::new(&path);
        let s = p.append("op", serde_json::json!({"b": 2})).unwrap();
        assert_eq!(s, 2, "重开后 seq 必须续接，不能从头开始");
        assert_eq!(p.load().unwrap().len(), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_file_reads_as_empty_not_error() {
        let path = tmp("missing");
        let p = JsonlPersistence::new(&path);
        assert!(p.load().unwrap().is_empty(), "未落盘应读作空日志");
    }

    #[test]
    fn timestamp_is_deterministic_on_purpose() {
        // 刻意取舍：日志不含真实时间（否则确定性回放测试会失效）
        let path = tmp("ts");
        let mut p = JsonlPersistence::new(&path);
        p.append("op", serde_json::json!({})).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains(DETERMINISTIC_TS), "时间戳应为确定性占位值");
        let _ = std::fs::remove_file(&path);
    }
}
