//! `neo` —— multitool 入口
//!
//! ```
//! neo exec "<task>" [--mode <plan|confirm|default|auto-edit|full>] [--json] [--workspace <dir>]
//! neo serve          # Web 宿主（未实现）
//! neo                # TUI 宿主（未实现）
//! ```
//!
//! **main 只做参数解析与装配**，业务在 L2/L3、渲染在 L5 宿主。
//! 这是「宿主不含业务逻辑」的落地点：换宿主不改这里之外任何东西。

use dsh_config::Config;
use dsh_exec::{build_kernel, describe_mode, run_task, ExecOptions};
use dsh_protocol::Decision;
use dsh_protocol::ExecMode;
use std::path::PathBuf;
use std::sync::Arc;

const USAGE: &str = r#"neo —— 用 Rust 重构的编程 Agent 内核

用法：
  neo exec "<任务>" [选项]     无头跑一轮（真实模型 + 真实沙箱）
  neo serve                    启动 Web 宿主（未实现）
  neo                          启动 TUI 宿主（未实现）

exec 选项：
  --mode <plan|confirm|default|auto-edit|full>   执行模式（默认 default）
  --workspace <dir>            工作区（默认当前目录）
  --max-steps <n>              步数上限（默认 16）
  --allow-writes               无人值守时自动批准写操作（默认拒绝）
  --json                       输出 JSON（便于脚本消费）
  --provider <deepseek|mock|selftest>   模型后端（默认 deepseek）
                                selftest = 按脚本调用一次工具，验证完整链路（无需 key）

环境变量：
  DEEPSEEK_API_KEY             必需（provider=deepseek 时）
  DEEPSEEK_BASE_URL            可选，默认 api.deepseek.com
  DEEPSEEK_MODEL               可选，默认 deepseek-chat
"#;

fn parse_mode(s: &str) -> Result<ExecMode, String> {
    Ok(match s {
        "plan" => ExecMode::Plan,
        "confirm" => ExecMode::ConfirmBefore,
        "default" => ExecMode::Default,
        "auto-edit" => ExecMode::AutoEdit,
        "full" => ExecMode::FullAccess,
        other => return Err(format!("未知模式 {other}")),
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("exec") => cmd_exec(&args[1..]),
        Some("serve") => {
            eprintln!("[neo] Web 宿主尚未实现（见 docs/neo-plan/04-落地计划）");
            2
        }
        Some("help") | Some("--help") | Some("-h") | None => {
            println!("{USAGE}");
            0
        }
        Some(other) => {
            eprintln!("[neo] 未知子命令：{other}\n\n{USAGE}");
            2
        }
    };
    std::process::exit(code);
}

fn cmd_exec(args: &[String]) -> i32 {
    let mut opts = ExecOptions::default();
    let mut workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut provider = "deepseek".to_string();
    let mut task_parts: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => opts.json = true,
            "--allow-writes" => opts.on_approval = Decision::Allow,
            "--mode" => {
                i += 1;
                match args.get(i).map(|s| parse_mode(s)) {
                    Some(Ok(m)) => opts.mode = m,
                    Some(Err(e)) => {
                        eprintln!("[neo] {e}");
                        return 2;
                    }
                    None => {
                        eprintln!("[neo] --mode 需要一个值");
                        return 2;
                    }
                }
            }
            "--workspace" => {
                i += 1;
                match args.get(i) {
                    Some(p) => workspace = PathBuf::from(p),
                    None => {
                        eprintln!("[neo] --workspace 需要一个目录");
                        return 2;
                    }
                }
            }
            "--max-steps" => {
                i += 1;
                match args.get(i).and_then(|s| s.parse().ok()) {
                    Some(n) => opts.max_steps = n,
                    None => {
                        eprintln!("[neo] --max-steps 需要正整数");
                        return 2;
                    }
                }
            }
            "--provider" => {
                i += 1;
                match args.get(i) {
                    Some(p) => provider = p.clone(),
                    None => {
                        eprintln!("[neo] --provider 需要一个值");
                        return 2;
                    }
                }
            }
            other => task_parts.push(other.to_string()),
        }
        i += 1;
    }

    if task_parts.is_empty() {
        eprintln!("[neo] 缺少任务描述。示例：neo exec \"列出当前目录的文件\"");
        return 2;
    }
    opts.task = task_parts.join(" ");

    // ── 装配：provider / sandbox / persistence ──────────────────────────
    let model: Box<dyn dsh_core::ModelProvider> = match provider.as_str() {
        "deepseek" => match dsh_llm_deepseek::DeepSeekProvider::from_env() {
            Ok(p) => Box::new(p),
            Err(e) => {
                eprintln!("[neo] {e}");
                eprintln!("       设置后重试：export DEEPSEEK_API_KEY=sk-...");
                return 2;
            }
        },
        "mock" => Box::new(dsh_llm_deepseek::ScriptedProvider::text_only(
            "（mock provider）本回答由确定性桩产生，未调用真实模型。",
        )),
        // 链路自检：第 1 步调用 apply_patch 改文件，第 2 步收尾。
        // 用途是在**无 API key**时验证「模型 → 工具 → 真实落盘」整条链路。
        // 目标文件由任务描述里的路径决定：`--workspace` 下的 `selftest.txt`。
        "selftest" => Box::new(dsh_llm_deepseek::ScriptedProvider::scripted(
            vec![vec![dsh_llm_deepseek::tool_call(
                "apply_patch",
                serde_json::json!({
                    "path": "selftest.txt",
                    "new": "由 selftest provider 经 apply_patch 写入。\n",
                }),
            )]],
            // 中性措辞：本 provider 不知道工具是否成功（可能被沙箱拦），
            // 断言"已落盘"会在被拦时给出**与实际不符**的输出。
            "selftest 脚本执行完毕（工具是否成功见上方 [tool] 行与下方失败原因）。",
        )),
        other => {
            eprintln!("[neo] 未知 provider：{other}（可选 deepseek | mock | selftest）");
            return 2;
        }
    };

    let sandbox = Arc::new(dsh_sandbox_local::LocalSandbox::new(&workspace));
    let persistence = Box::new(dsh_session_local::JsonlPersistence::new(
        workspace.join(".neo/sessions/neo-cli.jsonl"),
    ));

    if !opts.json {
        eprintln!("[neo] 工作区 {}", workspace.display());
        eprintln!("[neo] 模式   {}", describe_mode(opts.mode));
        eprintln!(
            "[neo] 沙箱   {}",
            if dsh_core::SandboxBackend::supports(sandbox.as_ref(), dsh_config::resolve(opts.mode).sandbox) {
                "已启用（OS 级强制）"
            } else {
                "本平台无实现 → 受限档位将被拒绝（fail-closed）"
            }
        );
        eprintln!("[neo] 模型   {provider}\n");
    }

    let kernel = build_kernel(
        "neo-cli",
        &workspace,
        &opts,
        model,
        sandbox,
        persistence,
    );

    let (ok, output) = run_task(kernel, &opts);
    println!("{output}");
    if ok { 0 } else { 1 }
}

// 让 `Config` 与 `dsh_core` 在本 crate 内可见（装配需要它们的类型）
#[allow(unused_imports)]
use dsh_core as _dsh_core;
#[allow(unused_imports)]
use Config as _Config;
