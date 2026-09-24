//! 对外线协议的类型导出：JSON Schema + TypeScript。
//!
//! # 为什么要单独有一份「产物」
//!
//! Codex 的 `app-server-protocol` 用 `ts-rs` 生成 TypeScript 给 GUI 消费 ——
//! **契约是编译器/生成器从类型抽出来的，不是人手抄的第二份**。此前我们的
//! `Op`/`EventMsg` 只是同进程 Rust 类型：跨进程客户端只能对着日志猜字段。
//! 本模块把它们（以及 app-server 的信封与方法参数）导出成可校验的产物，
//! 并由门禁保证「类型改了、产物没跟上」会变红。
//!
//! # 刻意的边界
//!
//! - **只导出，不发明语义**：字段形状全部来自 `neo-protocol` 与 `jsonrpc`。
//! - **默认不进构建**：`schema` 特性关闭时本模块不存在，发布二进制零负担。
//! - **生成物入库**：`schema/` 下的 JSON / TS 是给客户端用的事实来源；
//!   重生成后必须与入库版本逐字节一致（`check_protocol_schema.py`）。

use neo_protocol::{
    ApprovalPolicy, ContextRef, Decision, EventMsg, ExecMode, Fact, FileChange, GoalPhase,
    GoalSnapshot, GoalSubtask, Op, RefKind, SandboxMode, SessionPatch, TodoEntry, TodoStatus,
    ToolOutput, SCHEMA_VERSION,
};
use schemars::schema_for;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ── app-server 线格式（信封 + 方法参数）─────────────────────────────────
//
// 参数体刻意**复用协议层类型**（`SessionPatch` / `Decision`），这里只补
// 「JSON-RPC 信封」与「扁平参数结构」——后者是方法表的按名传参形状，
// 与 `jsonrpc::allowed_keys` 一一对应。

/// JSON-RPC 2.0 请求。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct RpcRequest {
    pub jsonrpc: String,
    /// 请求 id（字符串或整数）。
    #[cfg_attr(feature = "schema", ts(type = "unknown"))]
    pub id: Value,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", ts(type = "unknown"))]
    pub params: Option<Value>,
}

/// JSON-RPC 2.0 通知（无 id）。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct RpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", ts(type = "unknown"))]
    pub params: Option<Value>,
}

/// JSON-RPC 2.0 成功响应。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct RpcResponse {
    pub jsonrpc: String,
    #[cfg_attr(feature = "schema", ts(type = "unknown"))]
    pub id: Value,
    #[cfg_attr(feature = "schema", ts(type = "unknown"))]
    pub result: Value,
}

/// JSON-RPC 2.0 错误对象（响应或错误响应里的 `error`）。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct RpcErrorObject {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", ts(type = "unknown"))]
    pub data: Option<Value>,
}

/// JSON-RPC 2.0 错误响应。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct RpcErrorResponse {
    pub jsonrpc: String,
    #[cfg_attr(feature = "schema", ts(type = "unknown"))]
    pub id: Value,
    pub error: RpcErrorObject,
}

/// `initialize` 参数。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct InitializeParamsWire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<ClientInfoWire>,
}

/// 客户端自述（仅诊断）。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct ClientInfoWire {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// 文本类参数（`turn/start` / `turn/begin`）。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct TextParams {
    pub text: String,
}

/// `command/exec` 参数。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct CommandParams {
    pub command: String,
    /// true = 启动持活 PTY 会话；缺省 = 一次性 Shell。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
}

/// 审批应答参数（`approval/respond` / `approval/respondStep`）。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct ApprovalParams {
    pub id: String,
    pub decision: Decision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `session/rewind` 参数。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct RewindParams {
    pub turns: usize,
}

/// `goal/set` 参数。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct GoalParams {
    pub goal: String,
}

/// `goal/pause` / `goal/resume` 参数。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct GoalIdParams {
    pub goal_id: String,
}

/// `user_input/respond` 参数。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct UserInputParams {
    pub id: String,
    pub response: String,
}

/// `turn/steer` 参数（导出骨架：线上可用 text 或 Codex input[]）。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct SteerParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Codex `input[]`：导出为 unknown（形状见 Codex UserInput oneOf）。
    #[serde(default, rename = "input", skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema", ts(type = "unknown"))]
    pub input: Option<Value>,
    #[serde(default, rename = "expectedTurnId", alias = "expected_turn_id", skip_serializing_if = "Option::is_none")]
    pub expected_turn_id: Option<String>,
    #[serde(default, rename = "threadId", alias = "thread_id", skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

/// `thread/name/set` 参数（Codex threadId+name）。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct NameSetParams {
    #[serde(alias = "threadId")]
    pub id: String,
    #[serde(alias = "name")]
    pub title: String,
}

/// `thread/items/list` / `thread/turns/list`。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct ItemsListParams {
    #[serde(default, alias = "threadId", skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// `thread/turns/list`（与 items 同形，单独类型便于文档）。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct TurnsListParams {
    #[serde(default, alias = "threadId", skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// `fs/readFile` 等路径参数。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct FsPathParams {
    pub path: String,
}

/// `fs/writeFile` 参数。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct FsWriteParams {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(default, rename = "dataBase64", alias = "data_base64", skip_serializing_if = "Option::is_none")]
    pub data_base64: Option<String>,
}

/// `command/exec/write` 参数。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct ExecWriteParams {
    pub session_id: String,
    pub data: String,
}

/// `command/exec/resize` 参数。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct ExecResizeParams {
    pub session_id: String,
    pub cols: u16,
    pub rows: u16,
}

/// `command/exec/terminate` 参数。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct ExecTerminateParams {
    pub session_id: String,
}

/// 事件通知的线形状：`{"seq", "kind", "payload"}`。
///
/// 载荷嵌在 `payload` 下（与 JSONL 会话日志同构），**不摊平** ——
/// `ApprovalRequest` 自带的 `kind` 与通知的事件名不会撞键。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct EventNotification {
    pub seq: u64,
    pub kind: String,
    #[cfg_attr(feature = "schema", ts(type = "unknown"))]
    pub payload: Value,
}

/// 会话库条目（`thread/list` / `thread/get` 的 result 元素）。
///
/// 字段对齐 `neo_session::SessionInfo`，但状态是**线格式字符串** ——
/// 客户端不该依赖 Rust 枚举的 Debug 形状。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct ThreadSummary {
    pub id: String,
    /// 标题（取自第一条用户消息；取不到则等于 id）
    pub title: String,
    /// 标题是否来自真实内容（false = id 兜底，UI 应弱化显示）
    pub has_title: bool,
    /// 日志条数（粗略规模）
    pub records: usize,
    /// 文件字节数
    #[cfg_attr(feature = "schema", ts(type = "number"))]
    pub bytes: u64,
    /// 累计改动（增, 删）；从未改动为 null。取自**最后一个** files_changed。
    pub changes: Option<(usize, usize)>,
    /// idle / failed / interrupted / empty
    pub state: String,
}

/// 一份方法表条目（握手响应里的 `methods` 的元素形状说明用）。
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, ts_rs::TS)]
pub struct MethodDoc {
    pub name: String,
    /// 无参方法为空数组。
    pub params: Vec<String>,
    /// 是否在握手前可用。
    pub handshake_only: bool,
}

/// 本模块导出的全部根类型（生成器遍历用，新增类型必须登记）。
pub const ROOT_TYPES: &[&str] = &[
    "Op",
    "EventMsg",
    "ToolOutput",
    "ContextRef",
    "RefKind",
    "Decision",
    "SessionPatch",
    "ExecMode",
    "SandboxMode",
    "ApprovalPolicy",
    "FileChange",
    "TodoEntry",
    "TodoStatus",
    "GoalPhase",
    "GoalSubtask",
    "GoalSnapshot",
    "RpcRequest",
    "RpcNotification",
    "RpcResponse",
    "RpcErrorObject",
    "RpcErrorResponse",
    "InitializeParamsWire",
    "ClientInfoWire",
    "TextParams",
    "CommandParams",
    "ApprovalParams",
    "UserInputParams",
    "SteerParams",
    "NameSetParams",
    "ItemsListParams",
    "TurnsListParams",
    "FsPathParams",
    "FsWriteParams",
    "ExecWriteParams",
    "ExecResizeParams",
    "ExecTerminateParams",
    "RewindParams",
    "GoalParams",
    "GoalIdParams",
    "EventNotification",
    "MethodDoc",
    "ThreadSummary",
    "Fact",
];

/// 组合 JSON Schema 文档（draft 2020-12，schemars 1.x 默认）。
pub fn json_schema_document() -> Value {
    let mut defs = serde_json::Map::new();
    macro_rules! add {
        ($ty:ty) => {{
            let s = schema_for!($ty);
            let v = serde_json::to_value(s).unwrap_or(Value::Null);
            let name = stringify!($ty).to_string();
            defs.insert(name, v);
        }};
    }
    add!(Op);
    add!(EventMsg);
    add!(ToolOutput);
    add!(ContextRef);
    add!(RefKind);
    add!(Decision);
    add!(SessionPatch);
    add!(ExecMode);
    add!(SandboxMode);
    add!(ApprovalPolicy);
    add!(FileChange);
    add!(TodoEntry);
    add!(TodoStatus);
    add!(GoalPhase);
    add!(GoalSubtask);
    add!(GoalSnapshot);
    add!(RpcRequest);
    add!(RpcNotification);
    add!(RpcResponse);
    add!(RpcErrorObject);
    add!(RpcErrorResponse);
    add!(InitializeParamsWire);
    add!(ClientInfoWire);
    add!(TextParams);
    add!(CommandParams);
    add!(ApprovalParams);
    add!(UserInputParams);
    add!(SteerParams);
    add!(NameSetParams);
    add!(ItemsListParams);
    add!(TurnsListParams);
    add!(FsPathParams);
    add!(FsWriteParams);
    add!(ExecWriteParams);
    add!(ExecResizeParams);
    add!(ExecTerminateParams);
    add!(RewindParams);
    add!(GoalParams);
    add!(GoalIdParams);
    add!(EventNotification);
    add!(MethodDoc);
    add!(ThreadSummary);
    add!(Fact);

    // 元数据 + 根引用表：客户端可以按名取到每个根类型的 schema。
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://proteus.local/schema/neo-appserver.schema.json",
        "title": "NEO app-server wire protocol",
        "description": format!(
            "NEO app-server 线协议契约（SCHEMA_VERSION={SCHEMA_VERSION}）。\
             字段形状来自 neo-protocol 与 jsonrpc，由 schemars 从 Rust 类型生成。"
        ),
        "x-protocol-version": SCHEMA_VERSION,
        "x-methods": crate::jsonrpc::method_table(),
        "x-error-codes": {
            "PARSE_ERROR": crate::jsonrpc::PARSE_ERROR,
            "INVALID_REQUEST": crate::jsonrpc::INVALID_REQUEST,
            "METHOD_NOT_FOUND": crate::jsonrpc::METHOD_NOT_FOUND,
            "INVALID_PARAMS": crate::jsonrpc::INVALID_PARAMS,
            "KERNEL_ERROR": crate::jsonrpc::KERNEL_ERROR,
            "NOT_INITIALIZED": crate::jsonrpc::NOT_INITIALIZED,
            "ALREADY_INITIALIZED": crate::jsonrpc::ALREADY_INITIALIZED,
            "VERSION_MISMATCH": crate::jsonrpc::VERSION_MISMATCH,
        },
        "x-notification": {
            "event": {
                "method": "event",
                "shape": "EventNotification"
            }
        },
        "x-roots": ROOT_TYPES,
        "$defs": Value::Object(defs),
    })
}

/// 方法参数形状（与 `jsonrpc::allowed_keys` 同源，生成时并入文档）。
pub fn method_docs() -> Vec<MethodDoc> {
    crate::jsonrpc::method_param_docs()
        .into_iter()
        .map(|(name, params)| MethodDoc {
            name: name.into(),
            params: params.iter().map(|s| s.to_string()).collect(),
            handshake_only: name == "initialize",
        })
        .collect()
}

/// TypeScript 源（单文件）。类型声明来自 `ts-rs` 的 `decl()`，
/// 根类型按 `ROOT_TYPES` 顺序输出；依赖类型由 ts-rs 在 decl 里内联/引用。
pub fn typescript_source() -> String {
    use ts_rs::TS;
    let mut out = String::new();
    out.push_str("/**\n");
    out.push_str(" * NEO app-server 线协议 —— 由 `cargo test -p neo-host-appserver --features schema export_schema` 生成。\n");
    out.push_str(" * 不要手改：改 Rust 类型后重新生成，否则门禁 check_protocol_schema.py 会红。\n");
    out.push_str(&format!(" * protocol version: {SCHEMA_VERSION}\n"));
    out.push_str(" */\n\n");
    out.push_str(&format!(
        "export const PROTOCOL_VERSION = {SCHEMA_VERSION} as const;\n\n"
    ));
    out.push_str("export type ProtocolVersion = typeof PROTOCOL_VERSION;\n\n");

    // 方法表与错误码（握手响应的机器可读镜像）
    let methods = crate::jsonrpc::method_table();
    out.push_str("/** initialize 响应里的 methods 全表。 */\n");
    out.push_str("export const METHODS = [\n");
    for m in &methods {
        out.push_str(&format!("  \"{m}\",\n"));
    }
    out.push_str("] as const;\n");
    out.push_str("export type MethodName = (typeof METHODS)[number];\n\n");

    out.push_str("/** JSON-RPC 服务端错误码（data 可带 allowed / unknown 等细节）。 */\n");
    out.push_str("export const ERROR_CODES = {\n");
    out.push_str(&format!(
        "  PARSE_ERROR: {},\n",
        crate::jsonrpc::PARSE_ERROR
    ));
    out.push_str(&format!(
        "  INVALID_REQUEST: {},\n",
        crate::jsonrpc::INVALID_REQUEST
    ));
    out.push_str(&format!(
        "  METHOD_NOT_FOUND: {},\n",
        crate::jsonrpc::METHOD_NOT_FOUND
    ));
    out.push_str(&format!(
        "  INVALID_PARAMS: {},\n",
        crate::jsonrpc::INVALID_PARAMS
    ));
    out.push_str(&format!(
        "  KERNEL_ERROR: {},\n",
        crate::jsonrpc::KERNEL_ERROR
    ));
    out.push_str(&format!(
        "  NOT_INITIALIZED: {},\n",
        crate::jsonrpc::NOT_INITIALIZED
    ));
    out.push_str(&format!(
        "  ALREADY_INITIALIZED: {},\n",
        crate::jsonrpc::ALREADY_INITIALIZED
    ));
    out.push_str(&format!(
        "  VERSION_MISMATCH: {},\n",
        crate::jsonrpc::VERSION_MISMATCH
    ));
    out.push_str("} as const;\n\n");

    out.push_str("/** 方法 → 合法参数键（与服务端严格校验一致）。 */\n");
    out.push_str("export const METHOD_PARAMS: Record<string, readonly string[]> = {\n");
    for d in method_docs() {
        let params: Vec<String> = d.params.iter().map(|p| format!("\"{p}\"")).collect();
        out.push_str(&format!("  \"{}\": [{}],\n", d.name, params.join(", ")));
    }
    out.push_str("};\n\n");

    // 根类型逐一导出。依赖类型都已在 ROOT_TYPES 里登记 —— 不走
    // `dependencies()` 递归：那会把生成顺序交给遍历序，产物容易抖动，
    // 而 ROOT_TYPES 本身就是「契约全集」的机器核对清单（有测试盯着）。
    let mut emitted = std::collections::BTreeSet::new();
    let mut decls: Vec<String> = Vec::new();
    macro_rules! emit {
        ($ty:ty) => {{
            let name = <$ty as TS>::name().to_string();
            if emitted.insert(name) {
                decls.push(<$ty as TS>::decl());
            }
        }};
    }
    emit!(Op);
    emit!(EventMsg);
    emit!(ToolOutput);
    emit!(ContextRef);
    emit!(RefKind);
    emit!(Decision);
    emit!(SessionPatch);
    emit!(ExecMode);
    emit!(SandboxMode);
    emit!(ApprovalPolicy);
    emit!(FileChange);
    emit!(TodoEntry);
    emit!(TodoStatus);
    emit!(GoalPhase);
    emit!(GoalSubtask);
    emit!(GoalSnapshot);
    emit!(RpcRequest);
    emit!(RpcNotification);
    emit!(RpcResponse);
    emit!(RpcErrorObject);
    emit!(RpcErrorResponse);
    emit!(InitializeParamsWire);
    emit!(ClientInfoWire);
    emit!(TextParams);
    emit!(CommandParams);
    emit!(ApprovalParams);
    emit!(UserInputParams);
    emit!(SteerParams);
    emit!(NameSetParams);
    emit!(ItemsListParams);
    emit!(TurnsListParams);
    emit!(FsPathParams);
    emit!(FsWriteParams);
    emit!(ExecWriteParams);
    emit!(ExecResizeParams);
    emit!(ExecTerminateParams);
    emit!(RewindParams);
    emit!(GoalParams);
    emit!(GoalIdParams);
    emit!(EventNotification);
    emit!(MethodDoc);
    emit!(ThreadSummary);
    emit!(Fact);

    // 稳定顺序：按名字排，避免 HashMap 遍历导致产物抖动
    // （decls 本身已按 ROOT 优先、依赖补充的顺序压入；再按内容排序会让
    //  "先声明后使用" 在 TS 的 type 提升下仍然合法 —— type 别名可前向引用。）
    decls.sort();
    for d in decls {
        // JSON 线上没有 bigint：u64/usize 经 serde_json 都是 number。
        // ts-rs 默认映射成 `bigint`，那会让客户端以为要接 BigInt ——
        // 契产物必须描述**线格式**，不是 Rust 内存布局。
        let d = d.replace("bigint", "number");
        // ts-rs 的 decl() 不带 export；单文件契约要能被 `import`。
        let d = if d.starts_with("type ") || d.starts_with("interface ") {
            format!("export {d}")
        } else {
            d
        };
        out.push_str(&d);
        out.push('\n');
    }
    out
}

/// 一次生成全部产物（内容，不落盘）。
pub fn generate_all() -> (String, String) {
    let schema = json_schema_document();
    let schema_text = {
        let v = serde_json::to_string_pretty(&schema).expect("schema 序列化");
        format!("{v}\n")
    };
    (schema_text, typescript_source())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 生成器必须覆盖 ROOT_TYPES 里的每一个名字 —— 否则"加了类型忘了登记"
    /// 会静默少一段契约。这里用 schema `$defs` 的键集合做机器核对。
    #[test]
    fn every_root_type_appears_in_schema_defs() {
        let doc = json_schema_document();
        let defs = doc.get("$defs").expect("$defs");
        for name in ROOT_TYPES {
            assert!(
                defs.get(*name).is_some(),
                "ROOT_TYPES 登记了 {name}，但 schema $defs 里没有 —— 漏了 add! 或名字不一致"
            );
        }
    }

    #[test]
    fn method_docs_cover_all_ops() {
        let docs = method_docs();
        // initialize + 17 个 Op + 10 个 control
        assert_eq!(docs.len(), 49, "方法表 initialize + 19 Op + 25 control + aliases");
        assert!(docs[0].handshake_only);
        assert_eq!(docs[0].name, "initialize");
        assert!(docs.iter().any(|d| d.name == "thread/resume"));
        assert!(docs.iter().any(|d| d.name == "thread/rename"));
        assert!(docs.iter().any(|d| d.name == "tools/list"));
        assert!(docs.iter().any(|d| d.name == "models/list"));
        assert!(docs.iter().any(|d| d.name == "git/info"));
        assert!(docs.iter().any(|d| d.name == "thread/goal/get"));
        assert!(docs.iter().any(|d| d.name == "user_input/respond"));
        assert!(docs.iter().any(|d| d.name == "command/exec/write"));
        assert!(docs.iter().any(|d| d.name == "turn/steer"));
        assert!(docs.iter().any(|d| d.name == "skills/list"));
        assert!(docs.iter().any(|d| d.name == "fs/readFile"));
        assert!(docs.iter().any(|d| d.name == "thread/items/list"));
    }

    /// 写盘：`cargo test -p neo-host-appserver --features schema export_schema -- --ignored`
    ///
    /// 为什么是 `#[ignore]`：默认 `cargo test` 不该改工作区。
    /// 只有人/门禁显式要求重生成时才写文件。
    #[test]
    #[ignore = "写文件：只有要更新 schema 产物时才跑（--ignored export_schema）"]
    fn export_schema() {
        let (schema, ts) = generate_all();
        // 输出目录可覆盖：门禁写到临时目录做比对，人手重生成写回入库目录。
        let root = match std::env::var_os("NEO_SCHEMA_OUT") {
            Some(dir) => std::path::PathBuf::from(dir),
            None => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schema"),
        };
        std::fs::create_dir_all(&root).expect("创建 schema/");
        std::fs::write(root.join("neo-appserver.schema.json"), &schema).expect("写 schema.json");
        std::fs::write(root.join("neo-appserver.ts"), &ts).expect("写 ts");
        // 方法参数表单独一份，方便门禁与人类 diff
        let methods = serde_json::to_string_pretty(&method_docs()).expect("方法表序列化");
        std::fs::write(root.join("methods.json"), format!("{methods}\n")).expect("写 methods.json");
        eprintln!("schema 产物已写入 {}", root.display());
    }
}
