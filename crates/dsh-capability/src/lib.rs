//! L3 CAPABILITY —— Tool / Skill / Subagent + MCP client
//!
//! # Shell-First（ADR-0005）
//!
//! 默认只注册最小工具集：`bash` + `apply_patch` + `request_user_input`。
//! 一个 shell 执行器 + 一个受控写入口，胜过数百个专用工具 ——
//! 前者可组合，后者的组合方式要等需求来才知道。
//!
//! # 沙箱是结构保证，不是约定
//!
//! 这三个工具都**没有直接执行进程的能力**：它们唯一能执行命令的入口是
//! [`ToolCtx::exec`]，而 `ToolCtx` 由内核注入并已绑定沙箱档位。
//! 因此"工具绕过沙箱"在类型层面就不可能 —— 这是内核第 2 条铁律的落地点。

use dsh_core::{CallKind, SandboxOutcome, Tool, ToolCtx, ToolRegistry};
use dsh_protocol::ToolOutput;
use serde_json::Value;
use std::sync::Arc;

/// 从参数里取一个字符串字段。
fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

/// 明确属于只读的命令首词。
const READ_COMMANDS: &[&str] = &[
    "cat", "ls", "grep", "rg", "find", "head", "tail", "wc", "stat", "file", "pwd", "echo",
    "which", "type", "env", "printenv", "git", "cargo", "npm", "pnpm", "node", "python3", "go",
    "rustc", "diff", "sort", "uniq", "awk", "sed", "jq", "tree", "du", "df", "date",
];

/// 明确的写操作标志（出现任一即判 Write，**优先级高于只读首词**）。
const WRITE_MARKERS: &[&str] = &[
    ">", ">>", "tee", "rm ", "mv ", "cp ", "mkdir", "rmdir", "touch", "chmod", "chown", "ln ",
    "truncate", "dd ", "sed -i", "git commit", "git push", "git reset", "git checkout",
    "cargo install", "npm i ", "npm install", "pnpm add",
];

/// Shell-First 核心：读文件用 cat、搜索用 grep/find、跑测试用测试运行器。
///
/// **分类按命令内容判定**而非固定值：`cat`/`grep`/`ls` 是只读，
/// `rm`/`mv`/重定向是写。分类错误的代价不对称 —— 把写误判为读会绕过审批，
/// 所以判定**偏向保守**（识别不出就归为 Write）。
pub struct BashTool;

impl Tool for BashTool {
    fn name(&self) -> &str { "bash" }

    fn describe(&self) -> String {
        "bash(cmd): 执行只读或本地命令。读文件用 cat，搜索用 grep/find，跑测试用测试运行器。".into()
    }

    fn call_kind(&self, args: &Value) -> CallKind {
        let cmd = arg_str(args, "cmd").unwrap_or("");
        // 写标志优先：`echo hi > f` 的首词是 echo（只读），但它显然在写。
        if WRITE_MARKERS.iter().any(|m| cmd.contains(m)) {
            return CallKind::Write;
        }
        let first = cmd.split_whitespace().next().unwrap_or("");
        if READ_COMMANDS.contains(&first) {
            CallKind::Read
        } else {
            // 识别不出 → 保守判 Write（宁可多问一次，不可漏放一次写）
            CallKind::Write
        }
    }

    fn execute(&self, args: &Value, ctx: &ToolCtx) -> ToolOutput {
        let Some(cmd) = arg_str(args, "cmd") else {
            return ToolOutput {
                exit_code: -1,
                stdout: String::new(),
                stderr: "缺少参数 cmd".into(),
                truncated: false,
            };
        };
        // 唯一执行入口：经沙箱。工具自己无法选择沙箱档位。
        match ctx.exec(cmd) {
            // truncated 必须如实上抛：把截断结果当成完整结果会误导模型。
            SandboxOutcome::Ran { stdout, truncated } => {
                ToolOutput { exit_code: 0, stdout, stderr: String::new(), truncated }
            }
            SandboxOutcome::Denied { reason } => {
                ToolOutput { exit_code: -1, stdout: String::new(), stderr: reason, truncated: false }
            }
        }
    }
}

/// 所有文件变更的唯一通道（T7 铁律）。
///
/// 存在意义：让"文件变更"成为**可审计的单点**。若允许用 shell 重定向写文件，
/// 变更就会散落在任意命令里，无法做 hunk 级审批，也无法回放。
pub struct ApplyPatchTool;

impl Tool for ApplyPatchTool {
    fn name(&self) -> &str { "apply_patch" }

    fn describe(&self) -> String {
        "apply_patch(path, diff): 修改文件的唯一方式。禁止用 shell 重定向写文件。".into()
    }

    // 无条件写：本工具的存在就是为了写。
    fn call_kind(&self, _args: &Value) -> CallKind { CallKind::Write }

    fn execute(&self, args: &Value, _ctx: &ToolCtx) -> ToolOutput {
        let Some(path) = arg_str(args, "path") else {
            return ToolOutput {
                exit_code: -1,
                stdout: String::new(),
                stderr: "缺少参数 path".into(),
                truncated: false,
            };
        };
        // 诚实边界：diff 的**解析与应用**属 M3 工作，当前仅校验参数形态。
        // 不返回成功 —— 避免"看起来改了其实没改"的假象。
        let _ = arg_str(args, "diff");
        ToolOutput {
            exit_code: -1,
            stdout: String::new(),
            stderr: format!("apply_patch 尚未实现落盘（path={path}）；本原型只固定契约与分类"),
            truncated: false,
        }
    }
}

pub struct RequestUserInputTool;

impl Tool for RequestUserInputTool {
    fn name(&self) -> &str { "request_user_input" }
    fn describe(&self) -> String { "request_user_input(prompt): 向用户提问。".into() }
    fn call_kind(&self, _args: &Value) -> CallKind { CallKind::Interactive }
    fn execute(&self, _args: &Value, _ctx: &ToolCtx) -> ToolOutput {
        // 诚实边界：真正的交互需要宿主的 `interactive_prompt` 能力（T6 覆盖），
        // 当前只固定分类与契约。
        ToolOutput {
            exit_code: -1,
            stdout: String::new(),
            stderr: "request_user_input 需宿主交互能力；本原型未接线".into(),
            truncated: false,
        }
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
