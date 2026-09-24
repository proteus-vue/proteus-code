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
//! 默认从日志里**第一条用户消息**取前若干字符。用户改名时追加一条
//! `kind=op` / `payload.set_title` 记录（仍写在同一 JSONL，不另开标题文件）：
//! 列举时自定义标题覆盖自动标题。取不到就退回 ID —— 诚实显示"不知道叫什么"。

use std::path::{Path, PathBuf};

// 状态枚举只有**一份定义**，在 `neo-session`（契据所在处）——
// 这里 re-export 而不是各定义一份：两份同名类型会让"trait 要这个、
// 实现给那个"变成一个纯粹的转换烦恼（实测踩到：编译报 expected
// neo_session::SessionState, found neo_session_store::SessionState）。
pub use neo_session::SessionState;

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
    /// **累计改动行数**（增, 删）。取自日志里**最后一个** `files_changed`。
    ///
    /// ⚠️ 取最后一个而不是把每条加起来：内核的 `FilesChanged` 是
    /// **覆盖语义**（它自己持有累计状态），逐条相加会把同一文件重复计入。
    pub changes: Option<(usize, usize)>,
    /// 结束状态（见 [`SessionState`]）。
    pub state: SessionState,
    /// 是否已归档（`thread/archive`；仍在同一 JSONL 内 append-only 标记）。
    pub archived: bool,
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
            let sum = read_summary(&path).unwrap_or_default();
            let auto = if sum.title.is_empty() { id.to_string() } else { sum.title };
            let title = sum.custom_title.unwrap_or(auto);
            out.push(SessionMeta {
                id: id.to_string(),
                title,
                records: sum.records,
                bytes,
                path,
                changes: sum.changes,
                state: sum.state,
                archived: sum.archived,
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

    /// 用户改名 / 归档：向会话 JSONL **追加**一条 `kind=op` 记录。
    ///
    /// `set_title` 与 `set_archived` 共用追加逻辑 —— 两种元数据都不另开
    /// 旁路文件，避免第二真相源。
    fn append_op(&self, id: &str, payload: serde_json::Value) -> std::io::Result<bool> {
        use std::io::Write;
        let path = self.path_for(id);
        if !path.exists() {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
        }
        let seq = tail_max_seq(&path).unwrap_or(0) + 1;
        let line = serde_json::json!({
            "v": 1u32,
            "ts": "1970-01-01T00:00:00Z",
            "seq": seq,
            "kind": "op",
            "payload": payload,
        });
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        writeln!(f, "{line}")?;
        f.flush()?;
        Ok(true)
    }

    /// 用户改名：向会话 JSONL **追加**一条 `op/set_title`（append-only，
    /// 不改写既有字节）。列举时该标题覆盖首条用户消息派生的自动标题。
    ///
    /// 文件尚未创建（新建会话还没写过任何 op）时**创建**日志并写入该条 ——
    /// `thread/create` 是惰性建文件的，否则刚建完的会话无法改名。
    /// 真正不存在的 id 由调用方先 `exists` / 对照当前会话再调本方法。
    ///
    /// 不维护旁路标题文件 —— 否则出现第二个真相源（删文件/改 id 时标题漂移）。
    pub fn set_title(&self, id: &str, title: &str) -> std::io::Result<bool> {
        let title = title.trim();
        if title.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "标题不能为空",
            ));
        }
        let title: String = title.chars().take(48).collect();
        self.append_op(id, serde_json::json!({ "set_title": { "title": title } }))
    }

    /// 归档 / 取消归档（Codex `thread/archive` / `thread/unarchive`）。
    pub fn set_archived(&self, id: &str, archived: bool) -> std::io::Result<bool> {
        if !self.path_for(id).exists() && !archived {
            return Ok(false);
        }
        self.append_op(id, serde_json::json!({ "set_archived": { "archived": archived } }))
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
/// 单遍扫出会话摘要：标题、条数、累计改动、结束状态。
///
/// **一次读完**（而不是为每个字段各扫一遍）：会话列表每次打开都要列全部会话，
/// 日志可能有几万行，多扫几遍是白白的 I/O。
#[derive(Default)]
struct Summary {
    title: String,
    /// 用户 `set_title` 自定义标题（优先于首条用户消息）
    custom_title: Option<String>,
    records: usize,
    /// 最后一个 `files_changed` 的合计（覆盖语义，见 `SessionMeta::changes`）
    changes: Option<(usize, usize)>,
    state: SessionState,
    archived: bool,
}

/// 文件里已出现的最大 seq（追加改名用；与 JsonlWriter 尾读同义）。
fn tail_max_seq(path: &Path) -> Option<u64> {
    let raw = std::fs::read_to_string(path).ok()?;
    let mut last = None;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(s) = v.get("seq").and_then(|s| s.as_u64()) {
            last = Some(s);
        }
    }
    last
}

fn read_summary(path: &Path) -> Option<Summary> {
    let raw = std::fs::read_to_string(path).ok()?;
    let mut out = Summary { state: SessionState::Empty, ..Default::default() };
    // 每轮是否已收尾。`turn_started` 置 false，`turn_complete` 置 true。
    // 初始为 true（"没有未收尾的轮"）。
    let mut settled = true;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        out.records += 1;
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let kind = v.get("kind").and_then(|k| k.as_str()).unwrap_or("");
        let Some(p) = v.get("payload") else { continue };

        if kind == "op" {
            // 自定义标题：后写覆盖先写（用户可多次改名）
            if let Some(t) = p
                .get("set_title")
                .and_then(|s| s.get("title"))
                .and_then(|t| t.as_str())
            {
                let t = t.trim();
                if !t.is_empty() {
                    out.custom_title = Some(t.chars().take(48).collect());
                }
                continue;
            }
            // 归档标记：后写覆盖
            if let Some(a) = p
                .get("set_archived")
                .and_then(|s| s.get("archived"))
                .and_then(|v| v.as_bool())
            {
                out.archived = a;
                continue;
            }
            // 自动标题：第一条带非空文本的 user_turn
            if out.title.is_empty() {
                if let Some(t) =
                    p.get("user_turn").and_then(|u| u.get("text")).and_then(|t| t.as_str())
                {
                    let t = t.trim();
                    if !t.is_empty() {
                        out.title = t.chars().take(48).collect();
                    }
                }
            }
            continue;
        }
        if kind != "event" {
            continue;
        }
        // event 的 payload 是 {"<变体名>": {...}} 的单键映射
        let Some((name, body)) = p.as_object().and_then(|m| m.iter().next()) else {
            continue;
        };
        match name.as_str() {
            "turn_started" => {
                settled = false;
                out.state = SessionState::Interrupted; // 暂定；收尾会覆盖
            }
            "turn_complete" => {
                settled = true;
                out.state = SessionState::Idle;
            }
            "error" => {
                // 错误之后若还有正常的轮次收尾，应以收尾为准
                out.state = SessionState::Failed;
            }
            "files_changed" => {
                if let Some(files) = body.get("files").and_then(|f| f.as_array()) {
                    let mut add = 0usize;
                    let mut del = 0usize;
                    for f in files {
                        add += f.get("additions").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
                        del += f.get("deletions").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
                    }
                    out.changes = Some((add, del));
                }
            }
            _ => {}
        }
    }
    // 收尾与错误同时存在时：**以收尾为准**（一轮正常结束就说明它跑完了，
    // 中间某个工具报错不等于会话失败）
    if settled && out.state == SessionState::Interrupted {
        out.state = SessionState::Empty;
    }
    Some(out)
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

    /// 写一个会话，事件序列由调用方给（用于构造各种"结束状态"）。
    fn write_events(store: &SessionStore, id: &str, user_text: &str, payloads: Vec<serde_json::Value>) {
        let path = store.path_for(id);
        let mut lines = vec![serde_json::json!({
            "v": 1, "ts": "1970-01-01T00:00:00Z", "seq": 1, "kind": "op",
            "payload": {"user_turn": {"text": user_text, "refs": []}}
        })
        .to_string()];
        for (i, payload) in payloads.into_iter().enumerate() {
            lines.push(
                serde_json::json!({
                    "v": 1, "ts": "1970-01-01T00:00:00Z", "seq": i + 2, "kind": "event",
                    "payload": payload
                })
                .to_string(),
            );
        }
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    }

    /// **覆盖语义**：`files_changed` 是累计快照，取最后一个而不是逐条相加 ——
    /// 相加会把同一文件重复计入（内核自己已持有累计状态）。
    #[test]
    fn changes_take_the_last_files_changed_not_the_sum() {
        let dir = tmpdir("changes");
        let store = SessionStore::open(&dir);
        write_events(
            &store,
            "s-1",
            "改文件",
            vec![
                serde_json::json!({"files_changed": {"files": [
                    {"path": "a.rs", "additions": 3, "deletions": 1}
                ]}}),
                // 第二次是累计后的快照：a.rs 变成 +5 -2
                serde_json::json!({"files_changed": {"files": [
                    {"path": "a.rs", "additions": 5, "deletions": 2}
                ]}}),
            ],
        );
        let m = store.list().into_iter().find(|m| m.id == "s-1").unwrap();
        assert_eq!(m.changes, Some((5, 2)), "应取最后一个快照，不是 8/3");
    }

    /// 一轮跑完（有 `turn_complete`）→ 正常结束。
    #[test]
    fn completed_turn_means_idle() {
        let dir = tmpdir("idle");
        let store = SessionStore::open(&dir);
        write_events(
            &store,
            "s-1",
            "任务",
            vec![
                serde_json::json!({"turn_started": {"turn_id": "t1"}}),
                serde_json::json!({"turn_complete": {"input_tokens": 1, "output_tokens": 2}}),
            ],
        );
        assert_eq!(store.list()[0].state, SessionState::Idle);
    }

    /// **悬空的 `turn_started`**（没有收尾）→ 未完成。
    /// 这是磁盘上的历史：进程早就不在了，所以**不能**显示成"运行中"。
    #[test]
    fn dangling_turn_started_means_interrupted_not_running() {
        let dir = tmpdir("interrupted");
        let store = SessionStore::open(&dir);
        write_events(
            &store,
            "s-1",
            "任务",
            vec![serde_json::json!({"turn_started": {"turn_id": "t1"}})],
        );
        assert_eq!(store.list()[0].state, SessionState::Interrupted);
    }

    #[test]
    fn error_marks_the_session_failed() {
        let dir = tmpdir("failed");
        let store = SessionStore::open(&dir);
        write_events(
            &store,
            "s-1",
            "任务",
            vec![serde_json::json!({"error": {"message": "炸了"}})],
        );
        assert_eq!(store.list()[0].state, SessionState::Failed);
    }

    /// 中间报错但最后正常收尾 → **以收尾为准**（不标失败）。
    /// 否则"工具报过错"会被当成"这次会话失败了"，而它其实跑完了。
    #[test]
    fn a_settled_turn_wins_over_an_earlier_error() {
        let dir = tmpdir("err-then-ok");
        let store = SessionStore::open(&dir);
        write_events(
            &store,
            "s-1",
            "任务",
            vec![
                serde_json::json!({"error": {"message": "某工具失败"}}),
                serde_json::json!({"turn_complete": {"input_tokens": 1, "output_tokens": 1}}),
            ],
        );
        assert_eq!(
            store.list()[0].state,
            SessionState::Idle,
            "收尾了就不该标失败"
        );
    }

    /// 完全没有轮次的会话 → `Empty`（不是"失败"，也不是"未完成"）。
    #[test]
    fn a_session_without_turns_is_empty() {
        // ⚠️ tag 不能叫 "empty" —— 那是既有测试 `empty_store_lists_nothing`
        // 的目录名，`tmpdir` 按 tag 命名且会先删目录，两个测试会互相清掉
        //（实测撞过一次）
        let dir = tmpdir("no-turns");
        let store = SessionStore::open(&dir);
        write_events(&store, "s-1", "只是提了个问题", vec![]);
        assert_eq!(store.list()[0].state, SessionState::Empty);
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
    fn archive_flag_round_trips_in_list() {
        let d = tmpdir("archive");
        let store = SessionStore::open(&d);
        write_events(&store, "s-1", "任务", vec![
            serde_json::json!({"turn_started": {"turn_id": "t1"}}),
            serde_json::json!({"turn_complete": {"input_tokens": 1, "output_tokens": 1}}),
        ]);
        assert!(!store.list()[0].archived, "默认未归档");
        store.set_archived("s-1", true).unwrap();
        assert!(store.list()[0].archived, "归档后应可见");
        store.set_archived("s-1", false).unwrap();
        assert!(!store.list()[0].archived, "取消归档应回到 false");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn set_title_overrides_auto_title_without_rewriting_history() {
        let d = tmpdir("rename");
        let s = SessionStore::open(&d);
        write_session(&s, "s-1", "自动标题来自首条消息", 1);
        let before = std::fs::read_to_string(s.path_for("s-1")).unwrap();
        assert!(s.set_title("s-1", "  手工改名  ").unwrap());
        let after = std::fs::read_to_string(s.path_for("s-1")).unwrap();
        assert!(after.starts_with(&before), "append-only：既有字节不得改写");
        let m = s.list().into_iter().find(|m| m.id == "s-1").unwrap();
        assert_eq!(m.title, "手工改名");
        assert!(m.has_title());
        // 空标题拒绝
        assert!(s.set_title("s-1", "   ").is_err());
        // 惰性 create：尚无文件时 set_title 仍应建出日志（当前会话场景）
        assert!(s.set_title("s-fresh", "刚创建").unwrap());
        let m2 = s.list().into_iter().find(|m| m.id == "s-fresh").unwrap();
        assert_eq!(m2.title, "刚创建");
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
