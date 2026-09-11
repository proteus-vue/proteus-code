//! `apply_patch` 的**真实落盘**验证（不再是空返回值）。
//!
//! 重点覆盖三种"静默改错"的风险：找不到 target、命中多处、沙箱该拒的没拒。
//! 这些是文件变更工具最危险的失败模式 —— 出错本身不可怕，
//! **模型以为改成功而实际改了别处**才可怕。

use dsh_capability::ApplyPatchTool;
use dsh_core::{Tool, ToolCtx};
use dsh_protocol::{ExecMode, SandboxMode};
use dsh_sandbox_local::LocalSandbox;
use serde_json::json;
use std::path::{Path, PathBuf};

fn tmpdir(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("neo-patch-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// 在给定沙箱档位下跑一次 apply_patch。
fn patch(ws: &Path, mode: SandboxMode, args: serde_json::Value) -> dsh_protocol::ToolOutput {
    let sb = LocalSandbox::new(ws);
    let ctx = ToolCtx { sandbox: &sb, mode, cwd: ws, max_output_bytes: 64 * 1024 };
    ApplyPatchTool.execute(&args, &ctx)
}

fn ok(out: &dsh_protocol::ToolOutput) -> bool { out.exit_code == 0 }

#[test]
fn creates_a_new_file_when_old_is_omitted() {
    let ws = tmpdir("create");
    let out = patch(
        &ws,
        SandboxMode::WorkspaceWrite,
        json!({"path": "new.txt", "new": "hello\n"}),
    );
    assert!(ok(&out), "应成功：{out:?}");
    assert_eq!(std::fs::read_to_string(ws.join("new.txt")).unwrap(), "hello\n");
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn creates_intermediate_directories() {
    let ws = tmpdir("mkdirs");
    let out = patch(
        &ws,
        SandboxMode::WorkspaceWrite,
        json!({"path": "a/b/c.txt", "new": "deep"}),
    );
    assert!(ok(&out), "多级目录应自动创建：{out:?}");
    assert!(ws.join("a/b/c.txt").exists());
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn replaces_exact_text_once() {
    let ws = tmpdir("replace");
    std::fs::write(ws.join("f.txt"), "let x = 1;\nlet y = 2;\n").unwrap();
    let out = patch(
        &ws,
        SandboxMode::WorkspaceWrite,
        json!({"path": "f.txt", "old": "let x = 1;", "new": "let x = 42;"}),
    );
    assert!(ok(&out), "{out:?}");
    assert_eq!(
        std::fs::read_to_string(ws.join("f.txt")).unwrap(),
        "let x = 42;\nlet y = 2;\n"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn refuses_when_old_text_is_not_found() {
    // 关键：找不到就必须**不改动并报错**，不能静默当成功
    let ws = tmpdir("notfound");
    let original = "content A\n";
    std::fs::write(ws.join("f.txt"), original).unwrap();

    let out = patch(
        &ws,
        SandboxMode::WorkspaceWrite,
        json!({"path": "f.txt", "old": "content B", "new": "X"}),
    );
    assert!(!ok(&out), "找不到 old 必须失败");
    assert!(out.stderr.contains("未找到"), "错误应说明未找到：{}", out.stderr);
    assert_eq!(
        std::fs::read_to_string(ws.join("f.txt")).unwrap(),
        original,
        "失败时**内容不得改动**"
    );
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn refuses_ambiguous_match_and_leaves_file_untouched() {
    // 命中多处却只有单次意图 → 必须拒绝，而不是随手改第一处
    let ws = tmpdir("ambiguous");
    let original = "dup\ndup\n";
    std::fs::write(ws.join("f.txt"), original).unwrap();

    let out = patch(
        &ws,
        SandboxMode::WorkspaceWrite,
        json!({"path": "f.txt", "old": "dup", "new": "X"}),
    );
    assert!(!ok(&out), "多处命中必须失败");
    assert!(out.stderr.contains("2 处"), "错误应报告命中数：{}", out.stderr);
    assert_eq!(std::fs::read_to_string(ws.join("f.txt")).unwrap(), original);
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn all_true_replaces_every_occurrence() {
    let ws = tmpdir("alltrue");
    std::fs::write(ws.join("f.txt"), "dup\ndup\n").unwrap();
    let out = patch(
        &ws,
        SandboxMode::WorkspaceWrite,
        json!({"path": "f.txt", "old": "dup", "new": "X", "all": true}),
    );
    assert!(ok(&out), "{out:?}");
    assert_eq!(std::fs::read_to_string(ws.join("f.txt")).unwrap(), "X\nX\n");
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn read_only_mode_denies_all_writes() {
    // 与 bash 受**同一套**约束：只读档下 apply_patch 也必须拒
    let ws = tmpdir("readonly");
    let out = patch(&ws, SandboxMode::ReadOnly, json!({"path": "x.txt", "new": "no"}));
    assert!(!ok(&out), "只读档必须拒绝");
    assert!(out.stderr.contains("只读"), "应说明是只读档拒绝：{}", out.stderr);
    assert!(!ws.join("x.txt").exists(), "被拒的写不得落盘");
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn workspace_write_denies_paths_outside_the_workspace() {
    // 关键漏洞检查：不能靠 apply_patch 绕过沙箱写工作区外
    let ws = tmpdir("escape");
    let target = "/tmp/neo-patch-escape-should-not-exist.txt";
    let _ = std::fs::remove_file(target);

    let out = patch(
        &ws,
        SandboxMode::WorkspaceWrite,
        json!({"path": target, "new": "escaped"}),
    );
    assert!(!ok(&out), "工作区外写入必须被拒");
    assert!(out.stderr.contains("工作区之外"), "错误应说明越界：{}", out.stderr);
    assert!(!Path::new(target).exists(), "越界写不得落盘");
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn missing_old_is_rejected_for_replace_intent() {
    let ws = tmpdir("emptyold");
    std::fs::write(ws.join("f.txt"), "x").unwrap();
    let out = patch(
        &ws,
        SandboxMode::WorkspaceWrite,
        json!({"path": "f.txt", "old": "", "new": "y"}),
    );
    assert!(!ok(&out), "空 old 应被拒（应省略 old 而非给空串）");
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn apply_patch_is_in_the_default_tool_set_and_classified_as_write() {
    use dsh_core::{CallKind, ToolRegistry};
    let mut reg = ToolRegistry::new();
    dsh_capability::register_defaults(&mut reg);
    let tool = reg.get("apply_patch").expect("apply_patch 应在默认工具集内");
    // 分类为 Write 是审批闸门生效的前提
    assert_eq!(tool.call_kind(&json!({})), CallKind::Write);

    // 顺带确认档位语义：Default 档下写需审批
    let res = dsh_config::resolve(ExecMode::Default);
    assert!(matches!(
        dsh_core::gate(CallKind::Write, res),
        dsh_core::GateDecision::Ask { .. }
    ));
}
