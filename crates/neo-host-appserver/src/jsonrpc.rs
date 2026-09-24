//! JSON-RPC 2.0 信封 + "方法 → `Op`" 映射。
//!
//! 本模块**只做协议转换**（JSON 文本 ⇄ 协议层类型），不碰线程、不碰 IO ——
//! 那样它才能被当纯函数测，而 `serve` 只负责把请求行喂进来、把响应行写出去。
//!
//! # 参数是谁的类型
//!
//! 参数体直接复用协议层类型（`SessionPatch` / `Decision` / `ContextRef` 的
//! 字符串形式），**不在这里重写一套 DTO**：宿主自造一套参数类型，就等于在
//! 协议之外开了第二个契约，两边一旦漂移就是"客户端发的和内核收的不是一回事"。
//! 字段名沿用协议层的 `snake_case`（与 JSONL 日志、Web 宿主一致）。

use neo_protocol::{Decision, Op, SessionPatch, SCHEMA_VERSION};
use serde::Deserialize;
use serde_json::{json, Value};

/// 线上协议版本。取协议层的 `SCHEMA_VERSION` —— 握手报的就是它，
/// 客户端据此判断自己能不能跟这个内核说话。
pub const PROTOCOL_VERSION: u32 = SCHEMA_VERSION;

pub const JSONRPC_VERSION: &str = "2.0";

// 错误码。-32768..-32000 是 JSON-RPC 规范留给服务端实现的区间；
// 保留码（-32700/-32600/-32601/-32602）按规范语义使用。
/// 不是合法 JSON
pub const PARSE_ERROR: i32 = -32700;
/// 合法 JSON 但不是合法请求信封
pub const INVALID_REQUEST: i32 = -32600;
/// 方法不存在
pub const METHOD_NOT_FOUND: i32 = -32601;
/// 参数不合法（含未知字段 —— 拼错 `execMode` 必须报错，而不是静默忽略）
pub const INVALID_PARAMS: i32 = -32602;
/// 内核拒绝执行（提交通道已断等）
pub const KERNEL_ERROR: i32 = -32000;
/// 未握手：`initialize` 之外的任何方法在握手前一律拒绝
pub const NOT_INITIALIZED: i32 = -32001;
/// 重复握手
pub const ALREADY_INITIALIZED: i32 = -32002;
/// 协议版本不匹配（客户端报的版本 != 本内核的版本）
pub const VERSION_MISMATCH: i32 = -32003;

/// 一个 JSON-RPC 错误对象。
///
/// `data` 用于给机器可读的细节（哪个字段、服务端版本是多少）——
/// 客户端不该靠解析 `message` 文本分支。
#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

impl RpcError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), data: None }
    }

    pub fn with_data(code: i32, message: impl Into<String>, data: Value) -> Self {
        Self { code, message: message.into(), data: Some(data) }
    }

    /// 序列化成 JSON-RPC 的 `error` 成员。
    pub fn to_json(&self) -> Value {
        let mut obj = json!({ "code": self.code, "message": self.message });
        if let Some(data) = &self.data {
            obj["data"] = data.clone();
        }
        obj
    }
}

/// 客户端信息（`initialize` 里报的）。仅用于诊断与日志，不参与判定。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ClientInfo {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
pub struct InitializeParams {
    /// 客户端支持的协议版本。缺省 = 不声明（服务端按自己的版本继续，
    /// 并把版本号放在响应里让客户端自行决定要不要继续）。
    #[serde(default)]
    pub protocol_version: Option<u32>,
    #[serde(default)]
    pub client: Option<ClientInfo>,
}

/// 一条请求被解析后的**动作**。
///
/// 三种动作而不是"一律转成 `Op`"：握手要在本宿主内完成（内核不知道协议版本），
/// 关停要让 `serve` 知道"写完响应就停"。
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// 握手（本地处理，不下发内核）
    Initialize(InitializeParams),
    /// 下发一个 Op
    Submit(Op),
    /// 会话库控制（列举 / 读摘要 / 切换重建 / 新建 / 删除）。
    ///
    /// **不是 `Op`**：这些动作不驱动模型轮次，而是操作「会话库」这层
    /// （Codex 的 `thread/*` 同级）。放在线方法里而不再造一套 Op，
    /// 是因为 `Op` 的语义是"推进一次内核状态机"，而 list/get 是查询。
    Thread(ThreadCmd),
    /// 关停：下发 `Op::Shutdown`，并在响应之后结束读取循环
    Shutdown,
}

/// 会话库控制命令（`thread/*` 五个方法）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadCmd {
    /// `thread/list`：全部会话摘要，最近修改在前
    List,
    /// `thread/get`：单个会话摘要
    Get { id: String },
    /// `thread/resume`：切换到历史会话并重建事件流
    Resume { id: String },
    /// `thread/create`：新建空会话并切换过去
    Create,
    /// `thread/delete`：删除（不允许删当前会话）
    Delete { id: String },
    /// `thread/rename`：用户改名（同一 JSONL 追加 `op/set_title`）
    Rename { id: String, title: String },
    /// `thread/history`：历史投影（`facts_of` 的结果，与 T6 同源）。
    /// `id` 缺省 = 当前会话；给定 id 则只读投影，**不切换**。
    History { id: Option<String> },
    /// `thread/export`：导出对话（markdown / json），只读、不切换。
    Export { id: Option<String>, format: Option<String> },
    /// `tools/list`：工具目录（name / description / parameters）
    Tools,
    /// `models/list`：可选模型目录（桌面设置页的模型 picker 用）
    Models,
    /// `git/info`：工作区 git 元数据（branch / sha / origin_url）—— 零子进程
    GitInfo { cwd: Option<String> },
    /// `thread/goal/get`：当前目标完整快照（Codex 同名只读查询）
    GoalGet,
    /// `command/exec` + `session:true`：启动持活 PTY 会话（Codex unified_exec）
    ExecStart { command: String, cols: u16, rows: u16 },
    /// `command/exec/write`：向会话写 stdin（对齐 Codex unified_exec）
    ExecWrite { session_id: String, data: String },
    /// `command/exec/resize`：调整会话终端尺寸（PTY）
    ExecResize { session_id: String, cols: u16, rows: u16 },
    /// `command/exec/terminate`：结束会话
    ExecTerminate { session_id: String },
    /// `thread/name/set`（Codex：threadId+name）
    NameSet { id: String, title: String },
    /// `thread/items/list`：事实投影（与 history 同源，Codex items 形状）
    ItemsList { id: Option<String>, limit: Option<usize> },
    /// `thread/turns/list`：用户轮摘要
    TurnsList { id: Option<String>, limit: Option<usize> },
    /// `skills/list`
    SkillsList,
    /// `config/read`
    ConfigRead,
    /// `fs/readFile` — 绝对或相对工作区路径
    FsReadFile { path: String },
    /// `fs/writeFile`
    FsWriteFile { path: String, data: String, data_base64: Option<String> },
    /// `fs/getMetadata`
    FsGetMetadata { path: String },
    /// `fs/readDirectory`
    FsReadDirectory { path: String },
    /// `fs/copy`
    FsCopy { from: String, to: String },
    /// `fs/createDirectory`
    FsCreateDirectory { path: String },
    /// `fs/remove`
    FsRemove { path: String },
    /// `thread/archive` / `thread/unarchive`
    Archive { id: String, archived: bool },
    /// `hooks/list`（当前无 hooks 系统：如实空列表）
    HooksList,
    /// `mcpServerStatus/list`
    McpServerStatusList,
    /// `permissionProfile/list`（ExecMode 即权限档）
    PermissionProfileList,
    /// `modelProvider/capabilities/read`
    ModelProviderCapabilities,
    /// `thread/inject_items`：向当前会话历史追加用户可见文本（不驱动模型）
    InjectItems { text: String },
    /// `thread/revert`：按 beforeTurnId 回退（= rewind 若干用户轮）
    Revert { before_turn_id: String },
    /// `thread/metadata/update`：更新会话 git 元数据（append-only op）
    MetadataUpdate { id: String, branch: Option<String>, sha: Option<String>, origin_url: Option<String> },
    /// `thread/attachment/add`
    AttachmentAdd { id: String, attachment_type: String, identity_key: String, payload: Value },
    /// `thread/attachment/list`
    AttachmentList { id: String },
    /// `thread/attachment/remove`
    AttachmentRemove { id: String, attachment_type: String, identity_key: String },
    /// `mcpServer/tool/call`：经既有 Tool seam 调 mcp__server__tool
    McpToolCall { server: String, tool: String, arguments: Value },
    /// `mcpServer/resource/read`：经 mcp__server__read_resource
    McpResourceRead { server: String, uri: String },
    /// `marketplace/add`
    MarketplaceAdd { name: String, source: String },
    /// `marketplace/remove`
    MarketplaceRemove { name: String },
    /// `marketplace/upgrade`
    MarketplaceUpgrade { name: Option<String> },
    /// `plugin/list`
    PluginList,
    /// `plugin/read`
    PluginRead { name: String },
    /// `plugin/install`
    PluginInstall { name: String },
    /// `plugin/uninstall`
    PluginUninstall { id: String },
    /// `plugin/installed`
    PluginInstalled,
    /// `plugin/reconcile`
    PluginReconcile,
    /// `plugin/skill/read`
    PluginSkillRead { plugin: String, skill: String },
}

/// 除 `initialize` 外的全部方法名（按 `Op` 变体逐个对应，18 个）。
///
/// 这份表是**契约的一部分**：握手响应里原样返回它，客户端据此决定
/// 自己能用哪些能力，不需要读内核源码。
///
/// 另有两个 **Codex 名别名**（不占 Op 槽位，在 [`ALIAS_METHODS`]）：
/// `thread/goal/set` ≡ `goal/set`，`thread/goal/clear` ≡ `goal/clear`。
pub const OP_METHODS: &[&str] = &[
    "turn/start",
    "turn/begin",
    "turn/pump",
    "turn/interrupt",
    "turn/steer",
    "command/exec",
    "approval/respond",
    "approval/respondStep",
    "user_input/respond",
    "session/configure",
    "session/compact",
    "session/fork",
    "session/rewind",
    "goal/set",
    "goal/pause",
    "goal/resume",
    "goal/advance",
    "goal/clear",
    "shutdown",
];

/// Codex 方法名别名：映射到已有 `Op`，不增加 `Op` 变体数。
pub const ALIAS_METHODS: &[&str] = &[
    "thread/goal/set",
    "thread/goal/clear",
    "model/list",
    "thread/fork",
    "thread/compact/start",
];

/// 会话库控制方法（`ThreadCmd`）。**不是 Op 映射**，故不进 [`OP_METHODS`]。
///
/// 形态对齐 Codex app-server 的 `thread/*`：桌面左侧会话库靠它列举/切换/改名/删除。
pub const THREAD_METHODS: &[&str] = &[
    "thread/list",
    "thread/get",
    "thread/resume",
    "thread/create",
    "thread/start",
    "thread/delete",
    "thread/rename",
    "thread/name/set",
    "thread/history",
    "thread/read",
    "thread/export",
    "thread/goal/get",
    "thread/items/list",
    "thread/turns/list",
    "thread/archive",
    "thread/unarchive",
    "thread/loaded/list",
    "thread/inject_items",
    "thread/revert",
    "thread/metadata/update",
    "thread/attachment/add",
    "thread/attachment/list",
    "thread/attachment/remove",
    "mcpServer/tool/call",
    "mcpServer/resource/read",
    "tools/list",
    "models/list",
    "model/list",
    "git/info",
    "skills/list",
    "config/read",
    "hooks/list",
    "mcpServerStatus/list",
    "permissionProfile/list",
    "modelProvider/capabilities/read",
    "fs/readFile",
    "fs/writeFile",
    "fs/getMetadata",
    "fs/readDirectory",
    "fs/copy",
    "fs/createDirectory",
    "fs/remove",
    "command/exec/write",
    "command/exec/resize",
    "command/exec/terminate",
    "marketplace/add",
    "marketplace/remove",
    "marketplace/upgrade",
    "plugin/list",
    "plugin/read",
    "plugin/install",
    "plugin/uninstall",
    "plugin/installed",
    "plugin/reconcile",
    "plugin/skill/read",
];

/// 全部方法名（握手用）。
pub fn method_table() -> Vec<&'static str> {
    let mut v = vec!["initialize"];
    // model/list 同时在 THREAD 与 ALIAS 会重复 —— 只放一处。
    // THREAD 含 model/list（控制面），ALIAS 不再列它。
    v.extend_from_slice(OP_METHODS);
    v.extend_from_slice(THREAD_METHODS);
    for m in ALIAS_METHODS {
        if !THREAD_METHODS.contains(m) && !OP_METHODS.contains(m) {
            v.push(m);
        }
    }
    v
}

/// 方法 → 合法参数键（与 [`allowed_keys`] 同源，供 schema 导出）。
///
/// 单独开这个函数而不是直接暴露 `allowed_keys`：后者对未知方法返回 `None`
///（表示"不限制"），而契约导出需要**完整表**（含无参方法的空列表）。
pub fn method_param_docs() -> Vec<(&'static str, &'static [&'static str])> {
    let mut seen = std::collections::BTreeSet::new();
    let mut v = vec![("initialize", &["protocol_version", "client"][..])];
    seen.insert("initialize");
    for m in OP_METHODS {
        if seen.insert(m) {
            let keys = allowed_keys(m).unwrap_or(&[]);
            v.push((m, keys));
        }
    }
    for m in THREAD_METHODS {
        if seen.insert(m) {
            let keys = allowed_keys(m).unwrap_or(&[]);
            v.push((m, keys));
        }
    }
    for m in ALIAS_METHODS {
        if seen.insert(m) {
            let keys = allowed_keys(m).unwrap_or(&[]);
            v.push((m, keys));
        }
    }
    v
}

/// 每个方法的合法参数字段名。用于**严格校验**：出现表外的键即 `-32602`。
///
/// 为什么不用 `#[serde(deny_unknown_fields)]` 一把梭：`session/configure` 的
/// 参数体是协议层的 `SessionPatch`（它的 serde 属性属于 L0 契约，动它会牵动
/// 会话日志的形状）。在宿主侧统一做一次键检查，既保住了 L0 不动，又让
/// **所有方法**的错误行为一致（而不是有的严格、有的宽松）。
fn allowed_keys(method: &str) -> Option<&'static [&'static str]> {
    Some(match method {
        "initialize" => &["protocol_version", "client"],
        "turn/start" | "turn/begin" => &["text"],
        "turn/pump" | "turn/interrupt" | "session/compact" | "session/fork"
        | "goal/advance" | "goal/clear" | "thread/goal/clear" | "thread/goal/get"
        | "thread/fork" | "thread/compact/start" | "skills/list" | "config/read" | "shutdown" => &[],
        "turn/steer" => &["text", "expected_turn_id", "expectedTurnId", "thread_id", "threadId", "input"],
        "command/exec" => &["command", "session", "cols", "rows"],
        "approval/respond" | "approval/respondStep" => &["id", "decision", "reason"],
        "user_input/respond" => &["id", "response"],
        "session/configure" => &[
            "exec_mode",
            "sandbox_mode",
            "approval_policy",
            "model",
            "token_budget",
        ],
        "session/rewind" => &["turns"],
        "goal/set" | "thread/goal/set" => &["goal"],
        "goal/pause" | "goal/resume" => &["goal_id"],
        "thread/get" | "thread/resume" | "thread/delete" => &["id"],
        "thread/rename" => &["id", "title"],
        "thread/name/set" => &["id", "title", "threadId", "thread_id", "name", "title"],
        "thread/history" | "thread/items/list" | "thread/turns/list" | "thread/read" => &["id", "threadId", "thread_id", "limit"],
        "thread/export" => &["id", "format"],
        "thread/archive" | "thread/unarchive" => &["id", "threadId", "thread_id"],
        "thread/inject_items" => &["threadId", "thread_id", "text", "items"],
        "thread/revert" => &["threadId", "thread_id", "beforeTurnId", "before_turn_id"],
        "thread/metadata/update" => &["threadId", "thread_id", "gitInfo", "branch", "sha", "originUrl", "origin_url"],
        "thread/attachment/add" => &["threadId", "thread_id", "attachmentType", "attachment_type", "identityKey", "identity_key", "payload"],
        "thread/attachment/list" => &["threadId", "thread_id"],
        "thread/attachment/remove" => &["threadId", "thread_id", "attachmentType", "attachment_type", "identityKey", "identity_key"],
        "mcpServer/tool/call" => &["server", "tool", "threadId", "thread_id", "arguments", "_meta"],
        "mcpServer/resource/read" => &["server", "uri", "threadId", "thread_id", "target", "_meta"],
        "thread/list" | "thread/create" | "thread/start" | "thread/loaded/list"
        | "tools/list" | "models/list" | "model/list"
        | "hooks/list" | "mcpServerStatus/list" | "permissionProfile/list"
        | "modelProvider/capabilities/read" => &[],
        "git/info" => &["cwd"],
        "fs/readFile" | "fs/getMetadata" | "fs/readDirectory" | "fs/createDirectory" | "fs/remove" => &["path"],
        "fs/writeFile" => &["path", "data", "data_base64", "dataBase64"],
        "fs/copy" => &["from", "to", "source", "destination"],
        "marketplace/add" => &["name", "source", "refName", "sparsePaths"],
        "marketplace/remove" => &["marketplaceName", "name"],
        "marketplace/upgrade" => &["marketplaceName", "name"],
        "plugin/list" => &["cwds", "forceRefetch", "marketplaceKinds"],
        "plugin/installed" => &["cwds", "installSuggestionPluginNames"],
        "plugin/reconcile" => &["reason"],
        "plugin/read" | "plugin/install" => &[
            "pluginName",
            "name",
            "plugin_name",
            "marketplacePath",
            "remoteMarketplaceName",
            "installAttemptId",
        ],
        "plugin/uninstall" => &["pluginId", "id"],
        "plugin/skill/read" => &["pluginName", "plugin", "skillName", "skill", "remotePluginId", "remoteMarketplaceName"],
        "command/exec/write" => &["session_id", "data"],
        "command/exec/resize" => &["session_id", "cols", "rows"],
        "command/exec/terminate" => &["session_id"],
        _ => return None,
    })
}

fn invalid_params(message: impl Into<String>) -> RpcError {
    RpcError::new(INVALID_PARAMS, message)
}

/// 未知字段检查（严格契约：拼错的键必须报错，不能静默当"没传"）。
fn reject_unknown_keys(method: &str, params: &Value) -> Result<(), RpcError> {
    let Some(allowed) = allowed_keys(method) else {
        return Ok(());
    };
    let Some(map) = params.as_object() else {
        return Ok(()); // 非对象由各自的 from_value 报 -32602
    };
    let unknown: Vec<&str> = map
        .keys()
        .map(String::as_str)
        .filter(|k| !allowed.contains(k))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(RpcError::with_data(
        INVALID_PARAMS,
        format!("{method} 不认识的参数：{}", unknown.join(", ")),
        json!({ "unknown": unknown, "allowed": allowed }),
    ))
}

fn from_params<T: for<'de> Deserialize<'de>>(method: &str, params: &Value) -> Result<T, RpcError> {
    serde_json::from_value(params.clone())
        .map_err(|e| invalid_params(format!("{method} 参数不合法：{e}")))
}

/// 把一条请求转成动作。纯函数：同样的输入永远同样的输出（可单测）。
pub fn dispatch(method: &str, params: &Value) -> Result<Action, RpcError> {
    if !OP_METHODS.contains(&method)
        && !THREAD_METHODS.contains(&method)
        && !ALIAS_METHODS.contains(&method)
        && method != "initialize"
    {
        return Err(RpcError::new(
            METHOD_NOT_FOUND,
            format!("不认识的方法：{method}"),
        ));
    }
    // 本协议只接受**按名传参**：连无参方法也必须传对象（缺省 = `{}`）。
    // JSON-RPC 允许位置参数数组，但那条路会让"参数个数"变成第二个契约，
    // 且客户端少传一个参数时错位得无声无息 —— 直接拒绝。
    if !params.is_object() {
        return Err(invalid_params(format!(
            "{method} 的参数必须是对象（本协议按名传参，不支持位置参数数组）"
        )));
    }
    reject_unknown_keys(method, params)?;
    Ok(match method {
        "initialize" => Action::Initialize(from_params(method, params)?),
        // 引用（@file / $skill）由**协议层**解析，客户端不需要先解析再发：
        // 各宿主共用同一份 parse_refs，才不会漂移出"某个宿主不认 $ 技能"。
        "turn/start" => {
            let p: TextParams = from_params(method, params)?;
            Action::Submit(Op::UserTurn { refs: neo_protocol::parse_refs(&p.text), text: p.text })
        }
        "turn/begin" => {
            let p: TextParams = from_params(method, params)?;
            Action::Submit(Op::BeginTurn { refs: neo_protocol::parse_refs(&p.text), text: p.text })
        }
        "turn/pump" => Action::Submit(Op::Pump),
        "turn/interrupt" => Action::Submit(Op::Interrupt),
        "turn/steer" => {
            let p: SteerParams = from_params(method, params)?;
            let text = steer_text(&p)?;
            Action::Submit(Op::Steer { text })
        }
        "command/exec" => {
            let p: CommandParams = from_params(method, params)?;
            if p.session.unwrap_or(false) {
                Action::Thread(ThreadCmd::ExecStart {
                    command: p.command,
                    cols: p.cols.unwrap_or(80),
                    rows: p.rows.unwrap_or(24),
                })
            } else {
                Action::Submit(Op::Shell { command: p.command })
            }
        }
        // 两个审批方法只差变体名，字段完全一致 → 共用一个解析函数
        "approval/respond" => {
            let ApprovalFields { id, decision, reason } = approval(method, params)?;
            Action::Submit(Op::Approve { id, decision, reason })
        }
        "approval/respondStep" => {
            let ApprovalFields { id, decision, reason } = approval(method, params)?;
            Action::Submit(Op::ApproveStep { id, decision, reason })
        }
        "user_input/respond" => {
            let p: UserInputParams = from_params(method, params)?;
            Action::Submit(Op::RespondUserInput { id: p.id, response: p.response })
        }
        "session/configure" => {
            let patch: SessionPatch = from_params(method, params)?;
            Action::Submit(Op::ConfigureSession { patch })
        }
        "session/compact" => Action::Submit(Op::Compact),
        "session/fork" => Action::Submit(Op::Fork),
        "session/rewind" => {
            let p: RewindParams = from_params(method, params)?;
            Action::Submit(Op::Rewind { turns: p.turns })
        }
        "goal/set" | "thread/goal/set" => {
            let p: GoalParams = from_params(method, params)?;
            Action::Submit(Op::GoalSet { goal: p.goal })
        }
        "goal/pause" => {
            let p: GoalIdParams = from_params(method, params)?;
            Action::Submit(Op::GoalPause { goal_id: p.goal_id })
        }
        "goal/resume" => {
            let p: GoalIdParams = from_params(method, params)?;
            Action::Submit(Op::GoalResume { goal_id: p.goal_id })
        }
        "goal/advance" => Action::Submit(Op::GoalAdvance),
        "goal/clear" | "thread/goal/clear" => Action::Submit(Op::GoalClear),
        "thread/fork" => Action::Submit(Op::Fork),
        "thread/compact/start" => Action::Submit(Op::Compact),
        "thread/list" => Action::Thread(ThreadCmd::List),
        "thread/get" => {
            let p: IdParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::Get { id: p.id })
        }
        "thread/resume" => {
            let p: IdParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::Resume { id: p.id })
        }
        "thread/create" => Action::Thread(ThreadCmd::Create),
        "thread/delete" => {
            let p: IdParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::Delete { id: p.id })
        }
        "thread/rename" => {
            let p: RenameParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::Rename { id: p.id, title: p.title })
        }
        "thread/name/set" => {
            let p: NameSetParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::NameSet { id: p.id, title: p.title })
        }
        "thread/start" => Action::Thread(ThreadCmd::Create),
        "thread/goal/get" => Action::Thread(ThreadCmd::GoalGet),
        "thread/items/list" => {
            let p: ItemsListParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::ItemsList { id: p.id, limit: p.limit })
        }
        "thread/turns/list" => {
            let p: TurnsListParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::TurnsList { id: p.id, limit: p.limit })
        }
        "thread/read" => {
            let p: ThreadReadParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::History { id: Some(p.id) })
        }
        "thread/archive" => {
            let p: ThreadIdParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::Archive { id: p.id, archived: true })
        }
        "thread/unarchive" => {
            let p: ThreadIdParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::Archive { id: p.id, archived: false })
        }
        "thread/loaded/list" => Action::Thread(ThreadCmd::List),
        "thread/inject_items" => {
            let p: InjectItemsParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::InjectItems { text: inject_text(&p)? })
        }
        "thread/revert" => {
            let p: RevertParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::Revert { before_turn_id: p.before_turn_id })
        }
        "thread/metadata/update" => {
            let p: MetadataUpdateParams = from_params(method, params)?;
            let gi = p.git_info;
            let branch = p.branch.or_else(|| gi.as_ref().and_then(|g| g.get("branch").and_then(Value::as_str)).map(str::to_string));
            let sha = p.sha.or_else(|| gi.as_ref().and_then(|g| g.get("sha").and_then(Value::as_str)).map(str::to_string));
            let origin_url = p.origin_url.or_else(|| gi.as_ref().and_then(|g| g.get("origin_url").and_then(Value::as_str)).map(str::to_string));
            Action::Thread(ThreadCmd::MetadataUpdate { id: p.id, branch, sha, origin_url })
        }
        "thread/attachment/add" => {
            let p: AttachmentAddParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::AttachmentAdd {
                id: p.id,
                attachment_type: p.attachment_type,
                identity_key: p.identity_key,
                payload: p.payload,
            })
        }
        "thread/attachment/list" => {
            let p: AttachmentListParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::AttachmentList { id: p.id })
        }
        "thread/attachment/remove" => {
            let p: AttachmentRemoveParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::AttachmentRemove {
                id: p.id,
                attachment_type: p.attachment_type,
                identity_key: p.identity_key,
            })
        }
        "mcpServer/tool/call" => {
            let p: McpToolCallParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::McpToolCall {
                server: p.server,
                tool: p.tool,
                arguments: p.arguments.unwrap_or_else(|| json!({})),
            })
        }
        "mcpServer/resource/read" => {
            let p: McpResourceReadParams = from_params(method, params)?;
            let uri = resource_uri(&p)?;
            Action::Thread(ThreadCmd::McpResourceRead {
                server: p.server,
                uri,
            })
        }
        "skills/list" => Action::Thread(ThreadCmd::SkillsList),
        "config/read" => Action::Thread(ThreadCmd::ConfigRead),
        "hooks/list" => Action::Thread(ThreadCmd::HooksList),
        "mcpServerStatus/list" => Action::Thread(ThreadCmd::McpServerStatusList),
        "permissionProfile/list" => Action::Thread(ThreadCmd::PermissionProfileList),
        "modelProvider/capabilities/read" => Action::Thread(ThreadCmd::ModelProviderCapabilities),
        "fs/readFile" => {
            let p: FsPathParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::FsReadFile { path: p.path })
        }
        "fs/getMetadata" => {
            let p: FsPathParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::FsGetMetadata { path: p.path })
        }
        "fs/readDirectory" => {
            let p: FsPathParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::FsReadDirectory { path: p.path })
        }
        "fs/createDirectory" => {
            let p: FsPathParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::FsCreateDirectory { path: p.path })
        }
        "fs/remove" => {
            let p: FsPathParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::FsRemove { path: p.path })
        }
        "fs/copy" => {
            let p: FsCopyParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::FsCopy { from: p.from, to: p.to })
        }
        "fs/writeFile" => {
            let p: FsWriteParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::FsWriteFile {
                path: p.path,
                data: p.data.unwrap_or_default(),
                data_base64: p.data_base64,
            })
        }
        "model/list" => Action::Thread(ThreadCmd::Models),
        "marketplace/add" => {
            let p: MarketplaceAddParams = from_params(method, params)?;
            let name = p
                .name
                .or_else(|| {
                    std::path::Path::new(&p.source)
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                })
                .or(p.ref_name)
                .unwrap_or_else(|| "default".into());
            Action::Thread(ThreadCmd::MarketplaceAdd { name, source: p.source })
        }
        "marketplace/remove" => {
            let p: MarketplaceRemoveParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::MarketplaceRemove { name: p.name })
        }
        "marketplace/upgrade" => {
            let p: MarketplaceUpgradeParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::MarketplaceUpgrade { name: p.name })
        }
        "plugin/list" => Action::Thread(ThreadCmd::PluginList),
        "plugin/installed" => Action::Thread(ThreadCmd::PluginInstalled),
        "plugin/reconcile" => Action::Thread(ThreadCmd::PluginReconcile),
        "plugin/read" => {
            let p: PluginNameParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::PluginRead { name: p.name })
        }
        "plugin/install" => {
            let p: PluginNameParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::PluginInstall { name: p.name })
        }
        "plugin/uninstall" => {
            let p: PluginIdParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::PluginUninstall { id: p.id })
        }
        "plugin/skill/read" => {
            let p: PluginSkillReadParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::PluginSkillRead { plugin: p.plugin, skill: p.skill })
        }
        "command/exec/write" => {
            let p: ExecWriteParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::ExecWrite { session_id: p.session_id, data: p.data })
        }
        "command/exec/resize" => {
            let p: ExecResizeParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::ExecResize {
                session_id: p.session_id,
                cols: p.cols,
                rows: p.rows,
            })
        }
        "command/exec/terminate" => {
            let p: ExecTerminateParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::ExecTerminate { session_id: p.session_id })
        }
        "thread/history" => {
            let p: HistoryParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::History { id: p.id })
        }
        "thread/export" => {
            let p: ExportParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::Export { id: p.id, format: p.format })
        }
        "tools/list" => Action::Thread(ThreadCmd::Tools),
        "models/list" => Action::Thread(ThreadCmd::Models),
        "git/info" => {
            let p: GitInfoParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::GitInfo { cwd: p.cwd })
        }
        "shutdown" => Action::Shutdown,
        // 不可达：method 已在上面按 OP/THREAD/ALIAS 拦过。
        other => return Err(RpcError::new(METHOD_NOT_FOUND, format!("不认识的方法：{other}"))),
    })
}

/// 握手响应：客户端据此知道能不能用、有哪些能力、内核是哪一版。
pub fn initialize_result(params: &InitializeParams) -> Result<Value, RpcError> {
    if let Some(client_version) = params.protocol_version {
        if client_version != PROTOCOL_VERSION {
            return Err(RpcError::with_data(
                VERSION_MISMATCH,
                format!(
                    "协议版本不匹配：客户端 {client_version}，本内核 {PROTOCOL_VERSION}"
                ),
                json!({ "client": client_version, "server": PROTOCOL_VERSION }),
            ));
        }
    }
    Ok(json!({
        "protocol_version": PROTOCOL_VERSION,
        "server": { "name": "neo-app-server", "version": env!("CARGO_PKG_VERSION") },
        "methods": method_table(),
    }))
}

#[derive(Debug, Deserialize)]
struct TextParams {
    text: String,
}

#[derive(Debug, Deserialize)]
struct CommandParams {
    command: String,
    /// true = 启动持活 PTY 会话（返回 session_id）；缺省 = 一次性 Op::Shell。
    #[serde(default)]
    session: Option<bool>,
    #[serde(default)]
    cols: Option<u16>,
    #[serde(default)]
    rows: Option<u16>,
}

#[derive(Debug, Deserialize)]
struct RewindParams {
    turns: usize,
}

#[derive(Debug, Deserialize)]
struct GoalParams {
    goal: String,
}

#[derive(Debug, Deserialize)]
struct GoalIdParams {
    goal_id: String,
}

#[derive(Debug, Deserialize)]
struct IdParams {
    id: String,
}

/// `thread/rename` 参数。
#[derive(Debug, Deserialize)]
struct RenameParams {
    id: String,
    title: String,
}

/// `thread/history` 参数：`id` 可省（= 当前会话）。
#[derive(Debug, Deserialize)]
struct HistoryParams {
    #[serde(default)]
    id: Option<String>,
}

/// Codex `turn/steer`：兼容简单 `text` 与 Codex 形状（input[] / expectedTurnId）。
#[derive(Debug, Deserialize, Default)]
struct SteerParams {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    input: Option<serde_json::Value>,
    /// Codex 前置条件字段：dispatch 无内核句柄，仅**收下不报未知键**；
    /// 真正与 active turn 的比对在内核侧（装配可选校验）。
    #[allow(dead_code)]
    #[serde(default, alias = "expectedTurnId")]
    expected_turn_id: Option<String>,
    #[allow(dead_code)]
    #[serde(default, alias = "threadId")]
    thread_id: Option<String>,
}

fn inject_text(p: &InjectItemsParams) -> Result<String, RpcError> {
    if let Some(t) = p.text.as_deref() {
        if !t.trim().is_empty() {
            return Ok(t.to_string());
        }
    }
    if let Some(arr) = p.items.as_ref().and_then(Value::as_array) {
        let mut parts = Vec::new();
        for item in arr {
            if let Some(t) = item.get("text").and_then(Value::as_str) {
                parts.push(t);
            } else if let Some(t) = item.as_str() {
                parts.push(t);
            }
        }
        if !parts.is_empty() {
            return Ok(parts.join("\n"));
        }
    }
    Err(invalid_params("thread/inject_items 需要 text 或 items[].text"))
}

fn resource_uri(p: &McpResourceReadParams) -> Result<String, RpcError> {
    if let Some(u) = p.uri.as_deref() {
        if !u.trim().is_empty() {
            return Ok(u.to_string());
        }
    }
    if let Some(t) = p.target.as_ref() {
        if let Some(u) = t.get("uri").and_then(Value::as_str) {
            return Ok(u.to_string());
        }
    }
    Err(invalid_params("mcpServer/resource/read 需要 uri 或 target.uri"))
}

fn steer_text(p: &SteerParams) -> Result<String, RpcError> {
    if let Some(t) = p.text.as_deref() {
        if !t.trim().is_empty() {
            return Ok(t.to_string());
        }
    }
    if let Some(arr) = p.input.as_ref().and_then(Value::as_array) {
        let mut parts = Vec::new();
        for item in arr {
            if let Some(t) = item.get("text").and_then(Value::as_str) {
                parts.push(t);
            }
        }
        if !parts.is_empty() {
            return Ok(parts.join("\n"));
        }
    }
    Err(invalid_params("turn/steer 需要 text 或 input[].text"))
}

/// `thread/name/set`：Codex `threadId`+`name` → 我们的 id+title。
#[derive(Debug, Deserialize)]
struct NameSetParams {
    #[serde(alias = "threadId")]
    id: String,
    #[serde(alias = "name")]
    title: String,
}

/// `thread/items/list` / `thread/turns/list` 共用可选 id+limit。
#[derive(Debug, Deserialize)]
struct ItemsListParams {
    #[serde(default, alias = "threadId")]
    id: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct TurnsListParams {
    #[serde(default, alias = "threadId")]
    id: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

/// `fs/*` 路径参数。
#[derive(Debug, Deserialize)]
struct FsPathParams {
    path: String,
}

/// `fs/copy` 参数（Codex 用绝对路径 from/to；也接受 source/destination 别名）。
#[derive(Debug, Deserialize)]
struct FsCopyParams {
    #[serde(alias = "source")]
    from: String,
    #[serde(alias = "destination")]
    to: String,
}

/// Codex `thread/archive` / `thread/read` 的 threadId。
#[derive(Debug, Deserialize)]
struct ThreadIdParams {
    #[serde(alias = "threadId", alias = "thread_id")]
    id: String,
}

#[derive(Debug, Deserialize)]
struct ThreadReadParams {
    #[serde(alias = "threadId", alias = "thread_id")]
    id: String,
}


/// `thread/inject_items`：兼容 items[]（取首个 text）与简单 text 字段。
#[derive(Debug, Deserialize)]
struct InjectItemsParams {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    items: Option<Value>,
    #[serde(default, alias = "threadId", alias = "thread_id")]
    _thread: Option<String>,
}

/// `thread/revert`。
#[derive(Debug, Deserialize)]
struct RevertParams {
    #[serde(alias = "beforeTurnId", alias = "before_turn_id")]
    before_turn_id: String,
    #[serde(default, alias = "threadId", alias = "thread_id")]
    _thread: Option<String>,
}

/// `thread/metadata/update`。
#[derive(Debug, Deserialize)]
struct MetadataUpdateParams {
    #[serde(alias = "threadId", alias = "thread_id")]
    id: String,
    #[serde(default, alias = "gitInfo")]
    git_info: Option<Value>,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    sha: Option<String>,
    #[serde(default, alias = "originUrl")]
    origin_url: Option<String>,
}

/// `thread/attachment/add`。
#[derive(Debug, Deserialize)]
struct AttachmentAddParams {
    #[serde(alias = "threadId", alias = "thread_id")]
    id: String,
    #[serde(alias = "attachmentType", alias = "attachment_type")]
    attachment_type: String,
    #[serde(alias = "identityKey", alias = "identity_key")]
    identity_key: String,
    payload: Value,
}

#[derive(Debug, Deserialize)]
struct AttachmentListParams {
    #[serde(alias = "threadId", alias = "thread_id")]
    id: String,
}

#[derive(Debug, Deserialize)]
struct AttachmentRemoveParams {
    #[serde(alias = "threadId", alias = "thread_id")]
    id: String,
    #[serde(alias = "attachmentType", alias = "attachment_type")]
    attachment_type: String,
    #[serde(alias = "identityKey", alias = "identity_key")]
    identity_key: String,
}

/// `mcpServer/tool/call`。
#[derive(Debug, Deserialize)]
struct McpToolCallParams {
    server: String,
    tool: String,
    #[serde(default)]
    arguments: Option<Value>,
    #[serde(default, alias = "threadId")]
    _thread: Option<String>,
}

/// `mcpServer/resource/read`：Codex 用 target.uri 或顶层 uri。

/// `marketplace/add` 参数（Codex source 必填；name 可选时用目录名）。
#[derive(Debug, Deserialize)]
struct MarketplaceAddParams {
    source: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default, alias = "refName")]
    ref_name: Option<String>,
}

/// `marketplace/remove` / `upgrade`。
#[derive(Debug, Deserialize)]
struct MarketplaceRemoveParams {
    #[serde(alias = "marketplaceName")]
    name: String,
}

#[derive(Debug, Deserialize)]
struct MarketplaceUpgradeParams {
    #[serde(default, alias = "marketplaceName")]
    name: Option<String>,
}

/// `plugin/read` / `plugin/install`：pluginName 或 name。
#[derive(Debug, Deserialize)]
struct PluginNameParams {
    #[serde(alias = "pluginName", alias = "plugin_name")]
    name: String,
}

#[derive(Debug, Deserialize)]
struct PluginIdParams {
    #[serde(alias = "pluginId")]
    id: String,
}

/// `plugin/skill/read`。
#[derive(Debug, Deserialize)]
struct PluginSkillReadParams {
    #[serde(alias = "pluginName", alias = "plugin", alias = "remotePluginId")]
    plugin: String,
    #[serde(alias = "skillName", alias = "skill")]
    skill: String,
}

#[derive(Debug, Deserialize)]
struct McpResourceReadParams {
    server: String,
    #[serde(default)]
    uri: Option<String>,
    #[serde(default)]
    target: Option<Value>,
    #[serde(default, alias = "threadId")]
    _thread: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FsWriteParams {
    path: String,
    #[serde(default)]
    data: Option<String>,
    #[serde(default, alias = "dataBase64")]
    data_base64: Option<String>,
}

/// `thread/export` 参数。
#[derive(Debug, Deserialize)]
struct ExportParams {
    #[serde(default)]
    id: Option<String>,
    /// `markdown`（缺省）或 `json`
    #[serde(default)]
    format: Option<String>,
}

/// `git/info` 参数。
#[derive(Debug, Deserialize)]
struct GitInfoParams {
    #[serde(default)]
    cwd: Option<String>,
}

/// 审批应答的三个字段（两个方法共用）。`decision` 直接反序列化成协议层的
/// [`Decision`]，取值 `allow` / `allow_always` / `deny`。
#[derive(Debug, Deserialize)]
struct ApprovalParams {
    id: String,
    decision: Decision,
    #[serde(default)]
    reason: Option<String>,
}

/// 组装 `Op::Approve` / `Op::ApproveStep` 的字段。两个审批方法只差变体名，
/// 字段完全一致，故共用这一个解析结构。
struct ApprovalFields {
    id: String,
    decision: Decision,
    reason: Option<String>,
}

fn approval(method: &str, params: &Value) -> Result<ApprovalFields, RpcError> {
    let p: ApprovalParams = from_params(method, params)?;
    Ok(ApprovalFields { id: p.id, decision: p.decision, reason: p.reason })
}

/// `user_input/respond` 参数。
#[derive(Debug, Deserialize)]
struct UserInputParams {
    id: String,
    response: String,
}

/// `command/exec/write` 参数。
#[derive(Debug, Deserialize)]
struct ExecWriteParams {
    session_id: String,
    data: String,
}

/// `command/exec/resize` 参数。
#[derive(Debug, Deserialize)]
struct ExecResizeParams {
    session_id: String,
    cols: u16,
    rows: u16,
}

/// `command/exec/terminate` 参数。
#[derive(Debug, Deserialize)]
struct ExecTerminateParams {
    session_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_op_variant_has_exactly_one_method() {
        // 17 个方法对应协议层 17 个 Op 变体。数量写死在这里是**故意的**：
        // 协议层新增 `Op` 变体时，这个断言会红，逼着来补映射 ——
        // 否则新能力只在 TUI/桌面可用，线上永远发不出去（静默缺口）。
        assert_eq!(OP_METHODS.len(), 19, "Op 变体与方法数必须一一对应");
        let mut sorted = OP_METHODS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), OP_METHODS.len(), "方法名不得重复");
        assert!(ALIAS_METHODS.contains(&"model/list"));
        assert!(ALIAS_METHODS.contains(&"thread/fork"));
        assert!(ALIAS_METHODS.contains(&"thread/compact/start"));
        assert_eq!(ALIAS_METHODS.len(), 5, "Codex 别名应有 5 个");
    }

    #[test]
    fn dispatch_maps_methods_to_ops_with_protocol_parsed_refs() {
        let a = dispatch("turn/start", &json!({ "text": "看下 @src/main.rs" })).unwrap();
        match a {
            Action::Submit(Op::UserTurn { text, refs }) => {
                assert_eq!(text, "看下 @src/main.rs");
                assert_eq!(refs.len(), 1, "引用由协议层解析，客户端不必先解析再发");
                assert_eq!(refs[0].target, "src/main.rs");
            }
            other => panic!("turn/start 应产出 UserTurn：{other:?}"),
        }

        assert_eq!(dispatch("turn/pump", &json!({})).unwrap(), Action::Submit(Op::Pump));
        assert_eq!(dispatch("shutdown", &json!({})).unwrap(), Action::Shutdown);

        let a = dispatch(
            "approval/respond",
            &json!({ "id": "ap-1", "decision": "deny", "reason": "改错文件了" }),
        )
        .unwrap();
        match a {
            Action::Submit(Op::Approve { id, decision, reason }) => {
                assert_eq!(id, "ap-1");
                assert_eq!(decision, Decision::Deny);
                assert_eq!(reason.as_deref(), Some("改错文件了"));
            }
            other => panic!("approval/respond 应产出 Approve：{other:?}"),
        }
    }

    #[test]
    fn unknown_method_is_a_method_not_found_error() {
        let e = dispatch("turn/nope", &json!({})).unwrap_err();
        assert_eq!(e.code, METHOD_NOT_FOUND);
    }

    #[test]
    fn unknown_param_keys_are_rejected_instead_of_ignored() {
        // 拼成 camelCase 是最容易犯的错：必须报错，不能静默当没传
        let e = dispatch("session/configure", &json!({ "execMode": "default" })).unwrap_err();
        assert_eq!(e.code, INVALID_PARAMS);
        assert!(e.message.contains("execMode"), "错误消息要点名是哪个键：{}", e.message);
        // 合法参数照样通过
        assert!(dispatch("session/configure", &json!({ "exec_mode": "default" })).is_ok());
    }

    #[test]
    fn missing_required_params_are_invalid_params() {
        let e = dispatch("turn/start", &json!({})).unwrap_err();
        assert_eq!(e.code, INVALID_PARAMS);
        let e = dispatch("approval/respond", &json!({ "id": "ap-1" })).unwrap_err();
        assert_eq!(e.code, INVALID_PARAMS, "缺 decision 必须报错");
        // 非对象参数
        let e = dispatch("turn/pump", &json!([1, 2])).unwrap_err();
        assert_eq!(e.code, INVALID_PARAMS);
    }

    #[test]
    fn session_true_maps_to_exec_start_and_default_maps_to_shell() {
        match dispatch("command/exec", &json!({"command": "bash", "session": true})).unwrap() {
            Action::Thread(ThreadCmd::ExecStart { command, cols, rows }) => {
                assert_eq!(command, "bash");
                assert_eq!((cols, rows), (80, 24));
            }
            other => panic!("session:true 应产出 ExecStart：{other:?}"),
        }
        assert_eq!(
            dispatch("command/exec", &json!({"command": "ls"})).unwrap(),
            Action::Submit(Op::Shell { command: "ls".into() }),
        );
    }

    #[test]
    fn steer_accepts_text_and_codex_input_array() {
        match dispatch("turn/steer", &json!({ "text": "改用 Rust" })).unwrap() {
            Action::Submit(Op::Steer { text }) => assert_eq!(text, "改用 Rust"),
            other => panic!("{other:?}"),
        }
        match dispatch(
            "turn/steer",
            &json!({
                "expectedTurnId": "turn-1",
                "threadId": "s1",
                "input": [{"type": "text", "text": "先写测试"}]
            }),
        )
        .unwrap()
        {
            Action::Submit(Op::Steer { text }) => assert_eq!(text, "先写测试"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn marketplace_and_plugin_dispatch_without_account_params() {
        match dispatch("marketplace/add", &json!({"source": "/tmp/mkt"})).unwrap() {
            Action::Thread(ThreadCmd::MarketplaceAdd { name, source }) => {
                assert_eq!(source, "/tmp/mkt");
                assert_eq!(name, "mkt", "缺 name 时用 source 目录名");
            }
            other => panic!("{other:?}"),
        }
        match dispatch(
            "marketplace/remove",
            &json!({"marketplaceName": "local"}),
        )
        .unwrap()
        {
            Action::Thread(ThreadCmd::MarketplaceRemove { name }) => assert_eq!(name, "local"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            dispatch("plugin/list", &json!({"forceRefetch": true})).unwrap(),
            Action::Thread(ThreadCmd::PluginList)
        ));
        match dispatch("plugin/install", &json!({"pluginName": "demo"})).unwrap() {
            Action::Thread(ThreadCmd::PluginInstall { name }) => assert_eq!(name, "demo"),
            other => panic!("{other:?}"),
        }
        match dispatch("plugin/uninstall", &json!({"pluginId": "demo"})).unwrap() {
            Action::Thread(ThreadCmd::PluginUninstall { id }) => assert_eq!(id, "demo"),
            other => panic!("{other:?}"),
        }
        match dispatch(
            "plugin/skill/read",
            &json!({
                "remoteMarketplaceName": "local",
                "remotePluginId": "demo",
                "skillName": "demo"
            }),
        )
        .unwrap()
        {
            Action::Thread(ThreadCmd::PluginSkillRead { plugin, skill }) => {
                assert_eq!(plugin, "demo");
                assert_eq!(skill, "demo");
            }
            other => panic!("{other:?}"),
        }
        // 账号/分享键不在表内：必须点名拒绝，而不是静默吞掉
        let e = dispatch("plugin/list", &json!({"accessToken": "x"})).unwrap_err();
        assert_eq!(e.code, INVALID_PARAMS);
    }

    #[test]
    fn initialize_negotiates_protocol_version() {
        let ok = initialize_result(&InitializeParams {
            protocol_version: Some(PROTOCOL_VERSION),
            client: Some(ClientInfo { name: "vscode-neo".into(), version: Some("0.2".into()) }),
        })
        .unwrap();
        assert_eq!(ok["protocol_version"], PROTOCOL_VERSION);
        assert_eq!(
            ok["methods"].as_array().map(Vec::len),
            Some(79),
            "initialize + 19 Op + 55 control + aliases"
        );

        // 版本不匹配必须拒绝，且把双方版本放进 data（机器可读）
        let err = initialize_result(&InitializeParams {
            protocol_version: Some(PROTOCOL_VERSION + 1),
            client: None,
        })
        .unwrap_err();
        assert_eq!(err.code, VERSION_MISMATCH);
        assert_eq!(err.data.as_ref().unwrap()["server"], PROTOCOL_VERSION);

        // 不声明版本 = 接受（服务端在响应里报出版本，让客户端自行决定）
        let ok = initialize_result(&InitializeParams::default()).unwrap();
        assert_eq!(ok["protocol_version"], PROTOCOL_VERSION);
    }
}
