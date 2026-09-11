//! **真机验证** Seatbelt 是否真的拦住了越权写。
//!
//! 这不是 mock 测试 —— 它真的调用 `sandbox-exec` 起真进程。
//! 安全边界必须真机验证：一个只在 mock 里"生效"的沙箱等于没有沙箱。

#![cfg(target_os = "macos")]

use neo_core::{SandboxBackend, SandboxOutcome};
use neo_protocol::SandboxMode;
use neo_sandbox_local::LocalSandbox;

/// 读 stdout+「是否拒绝」的辅助。
fn run(sb: &LocalSandbox, mode: SandboxMode, cmd: &str) -> String {
    match sb.execute(mode, cmd, 64 * 1024) {
        SandboxOutcome::Ran { stdout, .. } => stdout,
        SandboxOutcome::Denied { reason } => format!("DENIED:{reason}"),
    }
}

#[test]
fn read_only_actually_blocks_writing_outside_workspace() {
    let ws = std::env::temp_dir().join("neo-sbx-test-ro");
    let _ = std::fs::create_dir_all(&ws);
    let sb = LocalSandbox::new(&ws).with_timeout(std::time::Duration::from_secs(20));

    let target = "/tmp/neo-sbx-should-not-exist";
    let _ = std::fs::remove_file(target);

    // 在 read-only 下尝试写一个工作区外的文件
    run(&sb, SandboxMode::ReadOnly, &format!("echo pwned > {target}"));

    // 真检查：文件不该存在
    assert!(
        !std::path::Path::new(target).exists(),
        "沙箱失效：read-only 档竟然写出了工作区外的文件 {target}"
    );
}

#[test]
fn workspace_write_allows_writing_inside_workspace() {
    let ws = std::env::temp_dir().join("neo-sbx-test-ws");
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(&ws).unwrap();
    let sb = LocalSandbox::new(&ws).with_timeout(std::time::Duration::from_secs(20));

    let out = run(&sb, SandboxMode::WorkspaceWrite, "echo hello > inside.txt && cat inside.txt");
    assert!(out.contains("hello"), "workspace-write 档应能写工作区内文件，实际输出：{out}");
    assert!(ws.join("inside.txt").exists(), "文件应真实落在工作区");
}

#[test]
fn workspace_write_blocks_writing_outside_workspace() {
    let ws = std::env::temp_dir().join("neo-sbx-test-ws2");
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(&ws).unwrap();
    let sb = LocalSandbox::new(&ws).with_timeout(std::time::Duration::from_secs(20));

    let target = "/tmp/neo-sbx-escape-attempt";
    let _ = std::fs::remove_file(target);
    run(&sb, SandboxMode::WorkspaceWrite, &format!("echo escaped > {target}"));
    assert!(
        !std::path::Path::new(target).exists(),
        "沙箱失效：workspace-write 档写出了工作区外的文件"
    );
}

#[test]
fn read_only_allows_reading() {
    let ws = std::env::temp_dir().join("neo-sbx-test-r");
    let _ = std::fs::create_dir_all(&ws);
    let sb = LocalSandbox::new(&ws).with_timeout(std::time::Duration::from_secs(20));
    let out = run(&sb, SandboxMode::ReadOnly, "echo readable");
    assert!(out.contains("readable"), "read-only 不应阻碍读取/执行，实际：{out}");
}

#[test]
fn output_cap_is_enforced_during_read_not_after() {
    let ws = std::env::temp_dir().join("neo-sbx-test-cap");
    let _ = std::fs::create_dir_all(&ws);
    let sb = LocalSandbox::new(&ws).with_timeout(std::time::Duration::from_secs(20));
    // 产出 1 MB，上限 1000 字节
    let out = run(&sb, SandboxMode::DangerFullAccess, "yes x | head -c 1000000");
    match sb.execute(SandboxMode::DangerFullAccess, "yes x | head -c 1000000", 1000) {
        SandboxOutcome::Ran { stdout, truncated } => {
            assert!(stdout.len() <= 1000, "输出未受限：{} 字节", stdout.len());
            assert!(truncated, "超限必须如实标记 truncated");
        }
        other => panic!("应能执行，实际：{other:?}"),
    }
    let _ = out;
}

#[test]
fn a_hanging_command_is_killed_by_timeout() {
    let ws = std::env::temp_dir().join("neo-sbx-test-to");
    let _ = std::fs::create_dir_all(&ws);
    let sb = LocalSandbox::new(&ws).with_timeout(std::time::Duration::from_millis(400));
    let started = std::time::Instant::now();
    let out = run(&sb, SandboxMode::DangerFullAccess, "sleep 30");
    let elapsed = started.elapsed();
    assert!(out.contains("超时"), "应如实上报超时，实际：{out}");
    assert!(elapsed < std::time::Duration::from_secs(5), "超时未生效，耗时 {elapsed:?}");
}
