//! `apply_patch` 的**真实落盘**验证（不再是空返回值）。
//!
//! 重点覆盖三种"静默改错"的风险：找不到 target、命中多处、沙箱该拒的没拒。
//! 这些是文件变更工具最危险的失败模式 —— 出错本身不可怕，
//! **模型以为改成功而实际改了别处**才可怕。

use neo_capability::ApplyPatchTool;
use neo_core::{Tool, ToolCtx};
use neo_protocol::{ExecMode, SandboxMode};
use neo_sandbox_local::LocalSandbox;
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
fn patch(ws: &Path, mode: SandboxMode, args: serde_json::Value) -> neo_protocol::ToolOutput {
    let sb = LocalSandbox::new(ws);
    let ctx = ToolCtx { sandbox: &sb, mode, cwd: ws, max_output_bytes: 64 * 1024 };
    ApplyPatchTool.execute(&args, &ctx)
}

fn ok(out: &neo_protocol::ToolOutput) -> bool { out.exit_code == 0 }

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
    use neo_core::{CallKind, ToolRegistry};
    let mut reg = ToolRegistry::new();
    neo_capability::register_defaults(&mut reg);
    let tool = reg.get("apply_patch").expect("apply_patch 应在默认工具集内");
    // 分类为 Write 是审批闸门生效的前提
    assert_eq!(tool.call_kind(&json!({})), CallKind::Write);

    // 顺带确认档位语义：Default 档下写需审批
    let res = neo_config::resolve(ExecMode::Default);
    assert!(matches!(
        neo_core::gate(CallKind::Write, res),
        neo_core::GateDecision::Ask { .. }
    ));
}

// ─────────── 预览与执行必须解析到**同一个文件** ───────────
//
// 这组测试守的是一个真实踩过的坑：`preview` 曾经按**进程当前目录**读相对路径，
// 而 `execute` 走 `ToolCtx::resolve` 按**工作区**解析。当 `--workspace` 不是进程
// CWD 时两者指向不同文件 —— 真机复现：工作区是 /tmp/x、CWD 是仓库根，仓库根
// 恰好有同名文件且内容相同，于是预览算出空 diff、界面什么都不显示，而用户仍被
// 要求批准一次真实写入。**"审批前看到改什么"这个保证就此失效。**

#[test]
fn preview_resolves_relative_paths_against_the_given_cwd() {
    let ws = tmpdir("preview-cwd");
    std::fs::write(ws.join("f.txt"), "KEEP\n").unwrap();

    // 相对路径：必须按传入的 cwd 读到 ws/f.txt（而不是进程 CWD）
    let (path, diff) = ApplyPatchTool
        .preview(&json!({"path": "f.txt", "new": "KEEP\nNEW\n"}), &ws)
        .expect("应能算出预览");
    assert_eq!(path, "f.txt", "预览里的路径保持用户给的形式（用于展示）");
    // KEEP 没被改动，所以它是**上下文行**（前导空格）而不是删除行。
    // 断言"它作为上下文出现在预览里"就足以证明读到的是 ws/f.txt 的旧内容 ——
    // 若读的是别处（比如进程 CWD 里同名但内容不同的文件），这里不会是 KEEP。
    // 只看**内容行**：`---`/`+++` 文件头也以 -/+ 开头，不能拿 `contains("-")` 判断
    let content: Vec<&str> = diff
        .lines()
        .filter(|l| !l.starts_with("---") && !l.starts_with("+++") && !l.starts_with("@@"))
        .collect();
    assert!(
        content.iter().any(|l| *l == " KEEP"),
        "预览应基于 ws/f.txt 的旧内容（KEEP 作为上下文行）：{diff}"
    );
    assert!(content.iter().any(|l| l.starts_with("+NEW")), "{diff}");
    assert!(
        !content.iter().any(|l| l.starts_with('-')),
        "只做新增，不该有删除行：{diff}"
    );
}

#[test]
fn preview_of_a_file_absent_from_the_workspace_is_a_full_addition() {
    // 这个用例是那次真机 bug 的**直接复现**：工作区里没有这个文件，
    // 所以预览必须是"整文件新增"（旧内容为空），不能因为别处有同名文件
    // 就算出一个空 diff 来。
    let ws = tmpdir("preview-absent");
    let (_, diff) = ApplyPatchTool
        .preview(
            &json!({"path": "only-here-please.txt", "new": "内容\n"}),
            &ws,
        )
        .expect("文件不存在也要给预览（这是整文件新增）");
    // ⚠️ 不能用 "包含 -" 判断：文件头 `--- a/...` 本身就以 `-` 开头。
    // 只看**内容行**（跳过 `---`/`+++`/`@@`）。
    let content_lines: Vec<&str> = diff
        .lines()
        .filter(|l| !l.starts_with("---") && !l.starts_with("+++") && !l.starts_with("@@"))
        .collect();
    assert!(
        !content_lines.iter().any(|l| l.starts_with('-')),
        "旧内容为空，不该出现删除行：{diff}"
    );
    assert!(
        content_lines.iter().any(|l| l.starts_with("+内容")),
        "应显示为新增：{diff}"
    );
    // 而且必须是"从 0 行开始"的纯新增（`@@ -0,0 +1,N @@`）
    assert!(
        diff.contains("@@ -0,0 "),
        "空旧文件的新增应以 -0,0 开头：{diff}"
    );
}

#[test]
fn preview_and_execute_agree_on_which_file_changes() {
    // 最强的一条：先取预览，再真执行，断言执行的实际结果与预览说的是同一件事。
    // 两者若解析到不同文件，这里必然对不上。
    let ws = tmpdir("preview-vs-exec");
    std::fs::write(ws.join("agree.txt"), "abc\n").unwrap();
    let args = json!({"path": "agree.txt", "old": "abc", "new": "XYZ"});

    let (_, diff) = ApplyPatchTool.preview(&args, &ws).expect("应有预览");
    assert!(diff.contains("-abc") && diff.contains("+XYZ"), "{diff}");

    let out = patch(&ws, SandboxMode::WorkspaceWrite, args);
    assert!(ok(&out), "执行应成功：{out:?}");
    // 预览说 ws/agree.txt 会从 abc 变成 XYZ —— 那就必须是这个文件变了
    assert_eq!(
        std::fs::read_to_string(ws.join("agree.txt")).unwrap(),
        "XYZ\n",
        "预览与实际改动必须作用在同一个文件上"
    );
}

#[test]
fn preview_returns_none_when_there_is_nothing_to_change() {
    // 内容相同 → 空 diff → 不给预览。这是对的（没有改动可展示），
    // 但**不能**因此把它和"解析到了别的文件"混为一谈 —— 上面两条用例
    // 保证的正是"文件不存在时会给出整文件新增的预览"。
    let ws = tmpdir("preview-noop");
    std::fs::write(ws.join("same.txt"), "SAME\n").unwrap();
    let got = ApplyPatchTool.preview(&json!({"path": "same.txt", "new": "SAME\n"}), &ws);
    assert!(got.is_none(), "无改动不该给预览：{got:?}");
}

