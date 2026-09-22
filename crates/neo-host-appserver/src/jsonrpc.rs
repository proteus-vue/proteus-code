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
    /// `thread/history`：历史投影（`facts_of` 的结果，与 T6 同源）。
    /// `id` 缺省 = 当前会话；给定 id 则只读投影，**不切换**。
    History { id: Option<String> },
    /// `thread/export`：导出对话（markdown / json），只读、不切换。
    Export { id: Option<String>, format: Option<String> },
    /// `tools/list`：工具目录（name / description / parameters）
    Tools,
    /// `git/info`：工作区 git 元数据（branch / sha / origin_url）—— 零子进程
    GitInfo { cwd: Option<String> },
}

/// 除 `initialize` 外的全部方法名（按 `Op` 变体逐个对应，17 个）。
///
/// 这份表是**契约的一部分**：握手响应里原样返回它，客户端据此决定
/// 自己能用哪些能力，不需要读内核源码。
pub const OP_METHODS: &[&str] = &[
    "turn/start",
    "turn/begin",
    "turn/pump",
    "turn/interrupt",
    "command/exec",
    "approval/respond",
    "approval/respondStep",
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

/// 会话库控制方法（`ThreadCmd`）。**不是 Op 映射**，故不进 [`OP_METHODS`]。
///
/// 形态对齐 Codex app-server 的 `thread/*`：桌面左侧会话库靠它列举/切换。
pub const THREAD_METHODS: &[&str] = &[
    "thread/list",
    "thread/get",
    "thread/resume",
    "thread/create",
    "thread/delete",
    "thread/history",
    "thread/export",
    "tools/list",
    "git/info",
];

/// 全部方法名（握手用）：`initialize` + 17 个 Op + 5 个 thread。
pub fn method_table() -> Vec<&'static str> {
    let mut v = vec!["initialize"];
    v.extend_from_slice(OP_METHODS);
    v.extend_from_slice(THREAD_METHODS);
    v
}

/// 方法 → 合法参数键（与 [`allowed_keys`] 同源，供 schema 导出）。
///
/// 单独开这个函数而不是直接暴露 `allowed_keys`：后者对未知方法返回 `None`
///（表示"不限制"），而契约导出需要**完整表**（含无参方法的空列表）。
pub fn method_param_docs() -> Vec<(&'static str, &'static [&'static str])> {
    let mut v = vec![("initialize", &["protocol_version", "client"][..])];
    for m in OP_METHODS {
        let keys = allowed_keys(m).unwrap_or(&[]);
        v.push((m, keys));
    }
    for m in THREAD_METHODS {
        let keys = allowed_keys(m).unwrap_or(&[]);
        v.push((m, keys));
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
        | "goal/advance" | "goal/clear" | "shutdown" => &[],
        "command/exec" => &["command"],
        "approval/respond" | "approval/respondStep" => &["id", "decision", "reason"],
        "session/configure" => &["exec_mode", "sandbox_mode", "approval_policy", "model"],
        "session/rewind" => &["turns"],
        "goal/set" => &["goal"],
        "goal/pause" | "goal/resume" => &["goal_id"],
        "thread/get" | "thread/resume" | "thread/delete" => &["id"],
        "thread/history" => &["id"],
        "thread/export" => &["id", "format"],
        "thread/list" | "thread/create" | "tools/list" => &[],
        "git/info" => &["cwd"],
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
        "command/exec" => {
            let p: CommandParams = from_params(method, params)?;
            Action::Submit(Op::Shell { command: p.command })
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
        "goal/set" => {
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
        "goal/clear" => Action::Submit(Op::GoalClear),
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
        "thread/history" => {
            let p: HistoryParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::History { id: p.id })
        }
        "thread/export" => {
            let p: ExportParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::Export { id: p.id, format: p.format })
        }
        "tools/list" => Action::Thread(ThreadCmd::Tools),
        "git/info" => {
            let p: GitInfoParams = from_params(method, params)?;
            Action::Thread(ThreadCmd::GitInfo { cwd: p.cwd })
        }
        "shutdown" => Action::Shutdown,
        // 不可达：method 已在上面按 OP_METHODS 拦过。
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

/// `thread/history` 参数：`id` 可省（= 当前会话）。
#[derive(Debug, Deserialize)]
struct HistoryParams {
    #[serde(default)]
    id: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_op_variant_has_exactly_one_method() {
        // 17 个方法对应协议层 17 个 Op 变体。数量写死在这里是**故意的**：
        // 协议层新增 `Op` 变体时，这个断言会红，逼着来补映射 ——
        // 否则新能力只在 TUI/桌面可用，线上永远发不出去（静默缺口）。
        assert_eq!(OP_METHODS.len(), 17, "Op 变体与方法数必须一一对应");
        let mut sorted = OP_METHODS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), OP_METHODS.len(), "方法名不得重复");
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
    fn initialize_negotiates_protocol_version() {
        let ok = initialize_result(&InitializeParams {
            protocol_version: Some(PROTOCOL_VERSION),
            client: Some(ClientInfo { name: "vscode-neo".into(), version: Some("0.2".into()) }),
        })
        .unwrap();
        assert_eq!(ok["protocol_version"], PROTOCOL_VERSION);
        assert_eq!(
            ok["methods"].as_array().map(Vec::len),
            Some(27),
            "initialize + 17 Op + 5 thread"
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
