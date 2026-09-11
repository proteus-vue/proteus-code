//! 工作区信任 —— 首次进入某目录前的知情同意（对标 MiMo Code / Codex）
//!
//! # 为什么沙箱之外还要这一层
//!
//! 沙箱回答"最多能做什么"（硬边界，OS 级强制）；信任回答"这个目录是不是
//! 你让我动的"（知情同意）。二者互补而不是重复：
//!   - 沙箱挡不住"我信任错了目录"——在错误目录里合法地改文件仍是灾难；
//!   - 信任挡不住"恶意代码越权"——那是沙箱的活。
//!
//! 典型场景：在下载来的第三方仓库里启动 Agent。沙箱允许写工作区，
//! 但用户未必知道那个仓库里的脚本会在工作区内做什么。
//!
//! # 为什么存到用户目录而不是项目里
//!
//! 存进度项目（如 `<ws>/.neo/trusted`）有两个问题：
//!   1. 污染仓库（要进 .gitignore，且每个新克隆都得重新问）；
//!   2. **可被仓库自己伪造** —— 恶意仓库只要带一个"已信任"文件就绕过了整道门。
//! 所以落在 `$NEO_HOME/trusted.json`（默认 `~/.neo/trusted.json`），
//! 由用户机器持有，仓库无法自称可信。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 信任库路径。`NEO_HOME` 可覆盖（测试与多用户隔离用）。
fn store_path() -> PathBuf {
    let home = std::env::var_os("NEO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".neo").join("trusted.json")
}

/// 规范化路径：消除符号链接、相对路径、尾斜杠带来的"同一目录两种写法"。
///
/// macOS 上 `/tmp` 与 `/private/tmp` 是两个字符串但同一目录 ——
/// 不规范化就会出现"信任了却还问"或"问过两次"。
pub fn normalize(ws: &Path) -> String {
    let p = std::fs::canonicalize(ws).unwrap_or_else(|_| {
        std::env::current_dir()
            .map(|c| c.join(ws))
            .unwrap_or_else(|_| ws.to_path_buf())
    });
    p.to_string_lossy().to_string()
}

/// 该工作区是否已被用户明确信任过。
pub fn is_trusted(ws: &Path) -> bool {
    load().contains(&normalize(ws))
}

/// 记录信任。写入失败要如实上报 —— 静默失败会导致"每次都问"，
/// 用户会以为是自己没点对。
pub fn trust(ws: &Path) -> std::io::Result<()> {
    let mut set = load();
    set.insert(normalize(ws));
    let path = store_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let list: Vec<&String> = set.iter().collect();
    let json = serde_json::to_string_pretty(&list).unwrap_or_else(|_| "[]".to_string());
    std::fs::write(&path, json)
}

/// 读取信任库。**任何解析失败都当"还没信任"**（fail-closed）：
/// 信任库损坏时宁可多问一次，也不能默认放行。
fn load() -> BTreeSet<String> {
    let Ok(raw) = std::fs::read_to_string(store_path()) else {
        return BTreeSet::new();
    };
    serde_json::from_str::<Vec<String>>(&raw)
        .map(|v| v.into_iter().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个用例用独立的 NEO_HOME，避免互相污染（环境变量是进程级的，
    /// 所以这些用例必须串行 —— 用一把锁保证）。
    fn with_home<T>(tag: &str, f: impl FnOnce(PathBuf) -> T) -> T {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("neo-trust-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("NEO_HOME", &dir);
        let out = f(dir.clone());
        std::env::remove_var("NEO_HOME");
        let _ = std::fs::remove_dir_all(&dir);
        out
    }

    #[test]
    fn unknown_workspace_is_not_trusted() {
        with_home("unknown", |home| {
            assert!(!is_trusted(&home.join("somewhere")));
        });
    }

    #[test]
    fn trusting_then_checking_round_trips() {
        with_home("roundtrip", |home| {
            let ws = home.join("proj");
            std::fs::create_dir_all(&ws).unwrap();
            assert!(!is_trusted(&ws), "初始不应已信任");
            trust(&ws).unwrap();
            assert!(is_trusted(&ws), "记录后应已信任");
        });
    }

    #[test]
    fn trust_survives_a_fresh_load() {
        // 信任必须落盘：重启进程后不能再问一次
        with_home("persist", |home| {
            let ws = home.join("proj");
            std::fs::create_dir_all(&ws).unwrap();
            trust(&ws).unwrap();
            // 直接读文件确认真的写下去了（而不是只改了内存里的集合）
            let raw = std::fs::read_to_string(store_path()).unwrap();
            assert!(raw.contains(&normalize(&ws)), "信任库文件里应含该路径：{raw}");
        });
    }

    #[test]
    fn corrupt_store_fails_closed() {
        with_home("corrupt", |home| {
            let ws = home.join("proj");
            std::fs::create_dir_all(&ws).unwrap();
            trust(&ws).unwrap();
            // 故意写坏信任库
            std::fs::write(store_path(), "{ 不是合法 JSON").unwrap();
            assert!(!is_trusted(&ws), "信任库损坏时必须当未信任（fail-closed）");
        });
    }

    #[test]
    fn different_paths_are_not_conflated() {
        with_home("distinct", |home| {
            let a = home.join("a");
            let b = home.join("b");
            std::fs::create_dir_all(&a).unwrap();
            std::fs::create_dir_all(&b).unwrap();
            trust(&a).unwrap();
            assert!(is_trusted(&a));
            assert!(!is_trusted(&b), "信任 a 不应连带信任 b");
        });
    }
}
