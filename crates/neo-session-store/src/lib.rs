//! L3 PROVIDER —— 会话库：多会话的**元信息、列举、新建、删除**
//!
//! # 与 `SessionPersistence` 的分工
//!
//! `SessionPersistence` 管**一个**会话的追加写（append-only 的核心契约）。
//! 会话库管**一批**会话：哪个目录、有哪些会话、每个的标题与规模、怎么新建/删除。
//! 二者是不同层次的关切，合成一个 trait 会让"单会话写入"的实现被迫知道
//! 目录布局 —— 那是把上层策略塞进了下层机制。
//!
//! # 会话 ID 的来源：文件名
//!
//! 一个会话 = 一个 JSONL 文件，**文件名（去扩展名）就是会话 ID**。
//! 好处是零额外索引：文件系统即索引，删除会话 = 删除文件，
//! 不会出现"索引说存在但文件没了"的不一致。
//! 代价是 ID 必须是安全的文件名 —— 由 `sanitize_id` 强制。
//!
//! # 标题从哪来
//!
//! 从日志里**第一条用户消息**取前若干字符。不额外维护标题文件：
//! 那会引入第二个真相源（改文件名不改标题、或反之）。
//! 取不到就退回 ID —— 诚实显示"不知道叫什么"，而不是编一个。

use std::path::{Path, PathBuf};

/// 一个会话的元信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMeta {
    /// 会话 ID（= 文件名去扩展名）
    pub id: String,
    /// 标题（取自第一条用户消息；取不到则等于 id）
    pub title: String,
    /// 日志条数（粗略反映规模）
    pub records: usize,
    /// 文件字节数
    pub bytes: u64,
    /// 文件路径
    pub path: PathBuf,
}

impl SessionMeta {
    /// 标题是否来自真实内容（否则是 ID 兜底）。
    pub fn has_title(&self) -> bool {
        self.title != self.id
    }
}

/// 会话库：一个目录下的所有会话。
pub struct SessionStore {
    root: PathBuf,
}

/// 会话 ID 的最大长度（防止超长文件名在不同文件系统上出问题）。
const MAX_ID_LEN: usize = 64;

impl SessionStore {
    /// 打开（不创建）会话库。目录不存在时视为空库 —— 首次运行不该报错。
    pub fn open(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 某会话的日志路径。
    pub fn path_for(&self, id: &str) -> PathBuf {
        self.root.join(format!("{}.jsonl", sanitize_id(id)))
    }

    /// 列出全部会话，按"最近修改优先"排序（最新的在最前，符合直觉）。
    ///
    /// 单个文件读失败**不中断整个列举** —— 一个坏文件不该让用户看不到
    /// 其它会话。该条以"未知标题 + 0 条记录"出现，用户仍能删除它。
    pub fn list(&self) -> Vec<SessionMeta> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut out: Vec<SessionMeta> = Vec::new();
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let meta = e.metadata().ok();
            let bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let (title, records) = read_title_and_count(&path).unwrap_or((id.to_string(), 0));
            out.push(SessionMeta {
                id: id.to_string(),
                title,
                records,
                bytes,
                path,
            });
        }
        // 最近修改优先；无法取时间的排在后面（按 id 稳定排序）
        out.sort_by_key(|m| {
            let t = std::fs::metadata(&m.path)
                .and_then(|md| md.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            // 逆序：时间大的在前（用 Reverse 语义，这里取负不行 —— u64，
            // 所以先按 `Reverse(t)` 排，再按 id 保证同秒稳定）
            (std::cmp::Reverse(t), m.id.clone())
        });
        out
    }

    /// 新建一个会话 ID。**不创建文件**（首次写入时惰性创建，
    /// 与 `JsonlPersistence` 的策略一致，避免"新建了却没用"留空文件）。
    ///
    /// 命名格式 `s-<纳秒低位>`：短、可读、基本不撞。
    /// 撞了就递增后缀 —— 用实际存在性判断，不靠概率。
    pub fn new_id(&self) -> String {
        let base = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0);
        for bump in 0..1000u32 {
            let id = format!("s-{}", (base + bump as u64) % 1_000_000_000);
            if !self.path_for(&id).exists() {
                return id;
            }
        }
        // 极端情况：全部撞上（不可能，但要有个确定的兜底）
        format!("s-{}", std::process::id())
    }

    /// 删除一个会话（连带其日志文件）。
    ///
    /// `found` 为 false 表示本来就不存在 —— 调用方据此告知用户，
    /// 而不是显示"已删除"（删除不存在的东西不该算成功）。
    pub fn delete(&self, id: &str) -> std::io::Result<bool> {
        let p = self.path_for(id);
        if !p.exists() {
            return Ok(false);
        }
        std::fs::remove_file(&p)?;
        Ok(true)
    }

    /// 会话是否存在。
    pub fn exists(&self, id: &str) -> bool {
        self.path_for(id).exists()
    }

    /// 会话数。
    pub fn len(&self) -> usize {
        self.list().len()
    }

    pub fn is_empty(&self) -> bool {
        self.list().is_empty()
    }
}

/// 把任意字符串规范成安全的文件名。
///
/// 为什么需要：会话 ID 可能来自外部（用户输入 / URL 参数）。若直接拼进
/// 路径，`../` 这类输入能写到目录之外 —— 这是路径穿越。
/// 这里只保留字母数字与 `-`/`_`，其余替换为 `_`，并限制长度。
pub fn sanitize_id(id: &str) -> String {
    let mut s: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    // 去掉开头可能的 `.`（避免 `.` / `..` 之类的特殊名）
    while s.starts_with('.') {
        s.remove(0);
    }
    if s.is_empty() {
        s.push_str("unnamed");
    }
    s.truncate(MAX_ID_LEN);
    s
}

/// 读标题（第一条用户消息）与记录条数。
fn read_title_and_count(path: &Path) -> Option<(String, usize)> {
    let raw = std::fs::read_to_string(path).ok()?;
    let mut title = None;
    let mut n = 0usize;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        n += 1;
        if title.is_some() {
            continue; // 只需要数条数了
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("kind").and_then(|k| k.as_str()) != Some("op") {
            continue;
        }
        // op 的 payload 是 Op 枚举；UserTurn 的 serde 表示是
        // {"user_turn":{"text":"...","refs":[...]}}
        let Some(p) = v.get("payload") else { continue };
        let text = p
            .get("user_turn")
            .and_then(|u| u.get("text"))
            .and_then(|t| t.as_str());
        if let Some(t) = text {
            let t = t.trim();
            if !t.is_empty() {
                title = Some(t.chars().take(48).collect::<String>());
            }
        }
    }
    let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    Some((title.unwrap_or_else(|| id.to_string()), n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("neo-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn write_session(store: &SessionStore, id: &str, user_text: &str, events: usize) {
        let path = store.path_for(id);
        let mut lines = vec![serde_json::json!({
            "v": 1, "ts": "1970-01-01T00:00:00Z", "seq": 1, "kind": "op",
            "payload": {"user_turn": {"text": user_text, "refs": []}}
        })
        .to_string()];
        for i in 0..events {
            lines.push(
                serde_json::json!({
                    "v": 1, "ts": "1970-01-01T00:00:00Z", "seq": i + 2, "kind": "event",
                    "payload": {"agent_message_done": {"text": "ok"}}
                })
                .to_string(),
            );
        }
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    }

    #[test]
    fn empty_store_lists_nothing_and_does_not_error() {
        let d = tmpdir("empty");
        let s = SessionStore::open(&d);
        assert!(s.list().is_empty(), "空目录应列出零个会话");
        assert!(s.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_directory_is_an_empty_store_not_a_panic() {
        let s = SessionStore::open("/definitely/not/here/neo-sessions");
        assert!(s.list().is_empty());
    }

    #[test]
    fn lists_sessions_with_title_from_first_user_message() {
        let d = tmpdir("list");
        let s = SessionStore::open(&d);
        write_session(&s, "s-1", "帮我重构一下解析器", 2);
        write_session(&s, "s-2", "写个测试", 0);
        let list = s.list();
        assert_eq!(list.len(), 2);
        let titles: Vec<&str> = list.iter().map(|m| m.title.as_str()).collect();
        assert!(titles.contains(&"帮我重构一下解析器"), "{titles:?}");
        assert!(titles.contains(&"写个测试"), "{titles:?}");
        let m = list.iter().find(|m| m.id == "s-1").unwrap();
        assert_eq!(m.records, 3, "1 条 op + 2 条 event");
        assert!(m.bytes > 0);
        assert!(m.has_title());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn newest_session_comes_first() {
        // 用户最可能想继续最近那个 —— 排序要符合直觉
        let d = tmpdir("order");
        let s = SessionStore::open(&d);
        write_session(&s, "s-old", "旧的", 0);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        write_session(&s, "s-new", "新的", 0);
        let list = s.list();
        assert_eq!(list[0].id, "s-new", "最近的应排最前：{:?}", list.iter().map(|m| &m.id).collect::<Vec<_>>());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_corrupt_file_falls_back_to_id_and_does_not_hide_others() {
        // 一个坏文件不该让用户看不到其它会话
        let d = tmpdir("corrupt");
        let s = SessionStore::open(&d);
        write_session(&s, "good", "好会话", 1);
        std::fs::write(s.path_for("bad"), "{ 这不是合法 JSONL").unwrap();
        let list = s.list();
        assert_eq!(list.len(), 2, "坏文件也要出现（可被删除）");
        let bad = list.iter().find(|m| m.id == "bad").unwrap();
        assert_eq!(bad.title, "bad", "取不到标题时退回 id");
        assert!(!bad.has_title());
        assert!(list.iter().any(|m| m.id == "good"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn new_id_does_not_collide_with_existing() {
        let d = tmpdir("newid");
        let s = SessionStore::open(&d);
        // 占住一批可能的名字，确保 new_id 仍能给出可用值
        let mut seen = std::collections::HashSet::new();
        for _ in 0..20 {
            let id = s.new_id();
            assert!(seen.insert(id.clone()), "new_id 不能重复：{id}");
            assert!(!s.exists(&id), "new_id 不该返回已存在的 id");
            std::fs::write(s.path_for(&id), "").unwrap();
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn new_id_is_a_safe_filename() {
        let d = tmpdir("safe");
        let s = SessionStore::open(&d);
        let id = s.new_id();
        assert_eq!(sanitize_id(&id), id, "new_id 产出的应当是安全文件名");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn delete_reports_whether_anything_was_removed() {
        let d = tmpdir("del");
        let s = SessionStore::open(&d);
        write_session(&s, "here", "存在的", 0);
        assert!(s.delete("here").unwrap(), "存在时应返回 true");
        assert!(!s.exists("here"));
        assert!(!s.delete("here").unwrap(), "已删除后再删应返回 false");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn sanitize_blocks_path_traversal() {
        // 安全边界：会话 ID 可能来自外部输入，绝不能拼出目录外的路径
        for evil in ["../etc/passwd", "..", ".", "a/../../b", "/abs/path"] {
            let safe = sanitize_id(evil);
            assert!(!safe.contains('/'), "{evil} → {safe} 仍含路径分隔符");
            assert!(!safe.contains(".."), "{evil} → {safe} 仍含 ..");
            assert!(!safe.starts_with('.'), "{evil} → {safe} 以 . 开头");
            assert!(!safe.is_empty());
        }
        // 正常名字应原样保留
        assert_eq!(sanitize_id("s-123456"), "s-123456");
        assert_eq!(sanitize_id("my_session"), "my_session");
    }

    #[test]
    fn sanitize_truncates_overlong_ids() {
        let long = "x".repeat(500);
        assert_eq!(sanitize_id(&long).len(), MAX_ID_LEN, "超长 id 应被截断");
    }

    #[test]
    fn ignores_non_jsonl_files() {
        let d = tmpdir("ext");
        let s = SessionStore::open(&d);
        std::fs::write(d.join("notes.txt"), "hi").unwrap();
        std::fs::write(d.join("backup.jsonl.bak"), "hi").unwrap();
        assert!(s.list().is_empty(), "只认 .jsonl");
        let _ = std::fs::remove_dir_all(&d);
    }
}
