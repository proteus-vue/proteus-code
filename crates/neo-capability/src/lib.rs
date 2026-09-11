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

use neo_core::{CallKind, SandboxOutcome, Tool, ToolCtx, ToolRegistry};
use neo_protocol::ToolOutput;
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
/// # 为什么是"精确文本替换"而不是 unified-diff 解析
///
/// 1. **语义无歧义**：`old` 必须唯一命中，否则报错。unified-diff 的上下文行
///    匹配有大量实现细节（忽略空白？偏移多少行？），每处模糊都可能**改错地方**。
/// 2. **失败响亮**：找不到 `old` 或命中多处 → 明确报错。静默改错文件比拒绝执行
///    危险得多（模型会以为改成功了）。
/// 3. **可被模型稳定产出**：要求模型给出行号偏移的 diff 是常见错误源。
///
/// # 契约
///
/// | 参数 | 行为 |
/// |---|---|
/// | `old` 省略 | 整文件写入（不存在则创建，存在则**覆盖**） |
/// | `old` 给出 | 精确替换。默认要求**恰好命中一次**；`all=true` 时替换全部 |
///
/// # 沙箱约束
///
/// 写入经 `ctx.write_file` —— 沙箱是唯一变更所有者。因此只读档会拒，
/// 工作区外的路径也会拒（与 `bash` 受同一套 OS 级约束，无漏洞）。
pub struct ApplyPatchTool;

/// 统计 `needle` 在 `hay` 中出现的次数（非重叠）。
fn count_occurrences(hay: &str, needle: &str) -> usize {
    if needle.is_empty() { return 0; }
    hay.match_indices(needle).count()
}

impl Tool for ApplyPatchTool {
    fn name(&self) -> &str { "apply_patch" }

    fn describe(&self) -> String {
        "apply_patch(path, old, new[, all]): 修改文件的唯一方式。\
         old 省略=整文件写入；给出=精确替换（默认须恰好命中一次）。\
         禁止用 shell 重定向写文件。"
            .into()
    }

    // 无条件写：本工具的存在就是为了写。
    fn call_kind(&self, _args: &Value) -> CallKind { CallKind::Write }

    fn execute(&self, args: &Value, ctx: &ToolCtx) -> ToolOutput {
        let Some(path_str) = arg_str(args, "path") else {
            return fail("缺少参数 path");
        };
        let Some(new) = args.get("new").and_then(Value::as_str) else {
            return fail("缺少参数 new（要写入的内容）");
        };
        let old = args.get("old").and_then(Value::as_str);
        let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);

        let path = ctx.resolve(path_str);

        // ── 组装最终内容 ───────────────────────────────────────────────
        let content: String = match old {
            // 整文件写入
            None => new.to_string(),
            // 精确替换
            Some(old_text) => {
                if old_text.is_empty() {
                    return fail("old 不能为空字符串（想整文件写入就省略 old）");
                }
                let existing = match std::fs::read_to_string(&path) {
                    Ok(s) => s,
                    Err(e) => {
                        return fail(&format!("无法读取 {}：{e}（替换必须先有文件）", path.display()))
                    }
                };
                let hits = count_occurrences(&existing, old_text);
                if hits == 0 {
                    return fail(&format!(
                        "old 在 {} 中未找到 —— 内容未改动。请先用 bash cat 确认当前文本",
                        path.display()
                    ));
                }
                if hits > 1 && !all {
                    return fail(&format!(
                        "old 在 {} 中命中 {hits} 处，无法确定改哪一处 —— 内容未改动。\
                         请给出更长的上下文使其唯一，或传 all=true 全部替换",
                        path.display()
                    ));
                }
                if all {
                    existing.replace(old_text, new)
                } else {
                    existing.replacen(old_text, new, 1)
                }
            }
        };

        // ── 落盘（唯一入口：经沙箱）─────────────────────────────────────
        match ctx.write_file(&path, &content) {
            neo_core::FileOutcome::Written { bytes } => ToolOutput {
                exit_code: 0,
                stdout: format!("已写入 {}（{bytes} 字节）", path.display()),
                stderr: String::new(),
                truncated: false,
            },
            neo_core::FileOutcome::Denied { reason } => fail(&format!("被沙箱拒绝：{reason}")),
            neo_core::FileOutcome::Failed { reason } => fail(&reason),
        }
    }
}

fn fail(msg: &str) -> ToolOutput {
    ToolOutput { exit_code: -1, stdout: String::new(), stderr: msg.to_string(), truncated: false }
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
