//! 把 MCP 服务器的工具适配成内核的 [`neo_core::Tool`]。
//!
//! 一个 MCP 服务器提供 N 个工具 → N 个 [`McpTool`] 实例，全部共享同一个
//! 连接（`Arc<Mutex<McpClient>>`）。工具名加 `mcp__<server>__` 前缀：
//! 服务器 A 和 B 都叫 `search` 时不至于互相覆盖 —— 前缀里的服务器名
//! 是命名空间，不是装饰。

use std::sync::{Arc, Mutex};

use neo_core::{CallKind, Tool, ToolCtx};
use neo_protocol::ToolOutput;
use serde_json::Value;

use crate::client::{CallOutcome, McpClient, McpError};

/// 工具名允许的字符。服务器给的名字可能含 `-`、`.`、`/`，
/// 而模型调用用的名字会进提示词与日志 —— 收敛成 `[A-Za-z0-9_]`。
fn sanitize(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

/// 一个 MCP 工具在内核里的形态。
pub struct McpTool {
    connection: Arc<Mutex<McpClient>>,
    qualified_name: String,
    description: String,
    input_schema: Value,
    read_only_hint: Option<bool>,
}

impl McpTool {
    /// 由服务器声明的工具信息构造。
    ///
    /// `connection` 由装配点创建并共享：一个服务器进程服务它的全部工具，
    /// 每个工具各起一个进程既浪费又会放大状态分叉。
    pub fn new(
        server_name: &str,
        info: &crate::client::ToolInfo,
        connection: Arc<Mutex<McpClient>>,
    ) -> Self {
        Self {
            connection,
            qualified_name: format!("mcp__{}__{}", sanitize(server_name), sanitize(&info.name)),
            description: info.description.clone(),
            input_schema: info.input_schema.clone(),
            read_only_hint: info.read_only_hint,
        }
    }

    fn failure(&self, e: McpError) -> ToolOutput {
        // 失败必须**可被模型理解**：exit -1 + 原因进 stderr，
        // 与本地工具的失败通道一致（4.28b：模型靠 stderr 理解失败）。
        ToolOutput { exit_code: -1, stdout: String::new(), stderr: e.to_string(), truncated: false }
    }
}

impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.qualified_name
    }

    /// 给模型看的说明。格式对齐内置工具（`名(参数): 说明`），
    /// 参数部分用服务器声明的 JSON Schema 原样内嵌 —— 它是**服务器
    /// 声明的契约**，客户端改写它只会引入两端理解不一致。
    /// 构造后字节不变（提示词缓存命中的前提）。
    fn describe(&self) -> String {
        let schema = self.input_schema.to_string();
        format!(
            "{}(MCP 参数 schema: {}): {}",
            self.qualified_name, schema, self.description
        )
    }

    /// 语义分类只信服务器**声明**的 `readOnlyHint`：
    /// 有 hint 且为 true → Read；没有 hint → 一律按 Write（保守判写，
    /// 因为把写误判成读会绕过审批 —— 与 bash 分类同一代价不对称）。
    fn call_kind(&self, _args: &Value) -> CallKind {
        match self.read_only_hint {
            Some(true) => CallKind::Read,
            _ => CallKind::Write,
        }
    }

    /// MCP 工具没有"改哪个文件"的结构化预览 —— 内容由服务器决定，
    /// 客户端无从生成可信 diff。不预览就不预览，不编一个假的。
    fn execute(&self, args: &Value, ctx: &ToolCtx) -> ToolOutput {
        let mut conn = self.connection.lock().unwrap_or_else(|e| e.into_inner());
        match conn.call_tool(&self.qualified_name, args) {
            Ok(CallOutcome { text, is_error }) => {
                // 双重上限：先按内核给的字节上限截断（UTF-8 安全），
                // 内核侧还会再兜一次（纵深防御，见 ToolCtx::exec 的同款设计）。
                let (text, cut) = truncate_utf8(&text, ctx.max_output_bytes);
                ToolOutput {
                    exit_code: if is_error { -1 } else { 0 },
                    stdout: text,
                    stderr: if is_error {
                        "工具自报执行失败（isError）".into()
                    } else {
                        String::new()
                    },
                    truncated: cut,
                }
            }
            Err(e) => self.failure(e),
        }
    }
}

/// 按字节上限截断，不切开 UTF-8 码点（`&s[..n]` 会 panic 的那种坑）。
fn truncate_utf8(s: &str, max: usize) -> (String, bool) {
    if s.len() <= max {
        return (s.to_string(), false);
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ToolInfo;

    fn info(name: &str, ro: Option<bool>) -> ToolInfo {
        ToolInfo {
            name: name.into(),
            description: "做一件事".into(),
            input_schema: serde_json::json!({"type": "object", "properties": {"q": {"type": "string"}}}),
            read_only_hint: ro,
        }
    }

    #[test]
    fn qualified_name_sanitizes_server_and_tool() {
        let conn = Arc::new(Mutex::new(never_client()));
        let t = McpTool::new("my-server.v2", &info("search/all", None), conn);
        assert_eq!(t.name(), "mcp__my_server_v2__search_all", "非法字符收敛为下划线");
    }

    #[test]
    fn describe_is_stable_and_embeds_schema() {
        let conn = Arc::new(Mutex::new(never_client()));
        let t = McpTool::new("srv", &info("q", Some(true)), conn);
        let d1 = t.describe();
        assert_eq!(d1, t.describe(), "两次调用必须逐字节相同");
        assert!(d1.contains(r#""type":"object""#), "schema 必须内嵌：{d1}");
        assert!(d1.contains("做一件事"));
    }

    #[test]
    fn call_kind_favors_write_without_an_explicit_hint() {
        let conn = Arc::new(Mutex::new(never_client()));
        assert_eq!(McpTool::new("s", &info("a", Some(true)), conn.clone()).call_kind(&Value::Null), CallKind::Read);
        assert_eq!(McpTool::new("s", &info("b", Some(false)), conn.clone()).call_kind(&Value::Null), CallKind::Write);
        assert_eq!(
            McpTool::new("s", &info("c", None), conn).call_kind(&Value::Null),
            CallKind::Write,
            "无 hint 一律按写（误判成读会绕过审批）"
        );
    }

    #[test]
    fn output_is_truncated_on_char_boundary() {
        let s = "约".repeat(100); // 每字 3 字节
        let (cut, truncated) = truncate_utf8(&s, 100);
        assert!(truncated);
        assert_eq!(cut.chars().count(), 33, "100 字节 = 33 个汉字（99 字节）");
        // 截断结果必须是合法 UTF-8（不 panic 即为证）
        assert_eq!(cut.len(), 99);
        let (_, t2) = truncate_utf8("short", 100);
        assert!(!t2);
    }

    #[test]
    fn tool_failure_carries_the_reason_in_stderr() {
        // 连接报错时，工具输出 exit -1 + stderr 带原因 —— 模型能理解失败
        let conn: Arc<Mutex<McpClient>> = Arc::new(Mutex::new(never_client()));
        let t = McpTool::new("s", &info("a", None), conn);
        // 不起真连接：直接验证 failure 的映射（通过手动构造 McpError）
        let out = t.failure(McpError::Server { code: -32601, message: "没有这个工具".into() });
        assert_eq!(out.exit_code, -1);
        assert!(out.stderr.contains("没有这个工具"), "{}", out.stderr);
    }

    /// 一个任何调用都会超时失败的桩客户端（不接真进程）。
    /// 用 `with_child` 塞一个空脚本桩：请求即 EOF。
    fn never_client() -> McpClient {
        use crate::client::mock::MockChild;
        McpClient::with_child(Box::new(MockChild::new(vec![])))
    }
}
