//! L3 CAPABILITY —— Tool / Skill / Subagent + MCP client
//!
//! 默认只注册最小工具集（Shell-First，见 ADR-0005）：bash + apply_patch + request_user_input。

use dsh_core::{Tool, ToolRegistry};
use dsh_protocol::ToolOutput;
use serde_json::Value;
use std::sync::Arc;

/// Shell-First 核心：cat 读 / grep、find 搜 / 跑测试 / 跑 linter
pub struct BashTool;
impl Tool for BashTool {
    fn name(&self) -> &str { "bash" }
    fn describe(&self) -> String {
        "bash(cmd): 执行只读或本地命令。读文件用 cat，搜索用 grep/find，跑测试用测试运行器。".into()
    }
    fn execute(&self, _args: &Value) -> ToolOutput {
        ToolOutput { exit_code: 0, stdout: String::new(), stderr: String::new(), truncated: false }
    }
}

/// 所有文件变更的唯一通道
pub struct ApplyPatchTool;
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str { "apply_patch" }
    fn describe(&self) -> String {
        "apply_patch(path, diff): 修改文件的唯一方式。禁止用 shell 重定向写文件。".into()
    }
    fn execute(&self, _args: &Value) -> ToolOutput {
        ToolOutput { exit_code: 0, stdout: String::new(), stderr: String::new(), truncated: false }
    }
}

pub struct RequestUserInputTool;
impl Tool for RequestUserInputTool {
    fn name(&self) -> &str { "request_user_input" }
    fn describe(&self) -> String { "request_user_input(prompt): 向用户提问。".into() }
    fn execute(&self, _args: &Value) -> ToolOutput {
        ToolOutput { exit_code: 0, stdout: String::new(), stderr: String::new(), truncated: false }
    }
}

/// 默认最小工具集。**新增工具必须显式声明，不得默认注册。**
pub fn register_defaults(reg: &mut ToolRegistry) {
    reg.register(Arc::new(BashTool));
    reg.register(Arc::new(ApplyPatchTool));
    reg.register(Arc::new(RequestUserInputTool));
}

/// Subagent（对齐 ZCode：Markdown 定义 + 工具白名单）
#[derive(Debug, Clone)]
pub struct SubagentSpec {
    pub name: String,
    pub model: String,
    pub tools: Vec<String>,
    pub body: String,
}
