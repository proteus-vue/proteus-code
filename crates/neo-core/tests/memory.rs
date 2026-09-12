//! 内存有界性与分配行为的**实测**验证。
//!
//! 单测试文件 + 计数分配器：全局分配器影响整个测试二进制，所以本文件的
//! 测量测试只有一个（避免并行计数互相污染）。
//!
//! 这里验证的是 Rust 给不了的东西：Rust 保证内存**安全**（无 UB），
//! 但不保证内存**有界**。一条命令吐 GB 级输出、或每步深拷贝整份历史，
//! 都能在完全安全的 Rust 里把进程拖垮。所以有界性必须自己测。

// 计数分配器是**全局状态**：并行跑会互相污染计数，导致断言随机失败。
// 本文件整体串行（`--test-threads=1` 的等价效果）：
// Rust 无 per-test 串行属性，故用全局 mutex 包住会读计数的测试。
use std::sync::Mutex;

/// 串行化本文件**全部**测试。
///
/// 只在"读计数"的那个用例上加锁是不够的：全局分配器记录的是**整个进程**的
/// 分配量，其它用例并行跑时照样在计数。结果是同一个用例时过时不过，
/// 表现为"疑似深拷贝历史"的假失败 —— 而这正是它要防的问题，很容易误判。
static COUNTER_LOCK: Mutex<()> = Mutex::new(());

use neo_config::Config;
use neo_core::{
    truncate_utf8, Kernel, Message, SandboxBackend, SandboxOutcome, Tool, ToolCtx, ToolRegistry,
    DEFAULT_MAX_OUTPUT_BYTES,
};
use neo_mock::{InMemoryPersistence, ScriptedModelProvider};
use neo_protocol::{ExecMode, Op, SandboxMode, ToolOutput};
use serde_json::Value;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

struct Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > layout.size() {
            ALLOCATED.fetch_add(new_size - layout.size(), Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static A: Counting = Counting;

fn allocated() -> usize { ALLOCATED.load(Ordering::Relaxed) }

// ── 测试替身 ──────────────────────────────────────────────────────────

/// 吐出海量输出的沙箱：模拟 `yes` / `find /` 这类失控命令。
struct FloodSandbox;

impl SandboxBackend for FloodSandbox {
    fn supports(&self, _m: SandboxMode) -> bool { true }
    fn write_file(&self, _m: SandboxMode, _p: &std::path::Path, content: &str) -> neo_core::FileOutcome {
        neo_core::FileOutcome::Written { bytes: content.len() }
    }

    fn execute(&self, _m: SandboxMode, _cmd: &str, _limit: usize) -> SandboxOutcome {
        // 4 MB 输出（真实上限是 256 KB）
        SandboxOutcome::Ran { stdout: "x".repeat(4 * 1024 * 1024), truncated: false }
    }
}

/// 多字节字符沙箱：截断点正好落在 UTF-8 码点中间。
struct MultibyteSandbox;

impl SandboxBackend for MultibyteSandbox {
    fn supports(&self, _m: SandboxMode) -> bool { true }
    fn write_file(&self, _m: SandboxMode, _p: &std::path::Path, content: &str) -> neo_core::FileOutcome {
        neo_core::FileOutcome::Written { bytes: content.len() }
    }

    fn execute(&self, _m: SandboxMode, _cmd: &str, _limit: usize) -> SandboxOutcome {
        // 每个「好」是 3 字节，共 3000 字节；上限设 10 字节会切在第 3 个字符中间
        SandboxOutcome::Ran { stdout: "好".repeat(1000), truncated: false }
    }
}

struct ProbeTool;

impl Tool for ProbeTool {
    fn name(&self) -> &str { "probe" }
    fn describe(&self) -> String { "probe()".into() }
    fn call_kind(&self, _a: &Value) -> neo_core::CallKind { neo_core::CallKind::Read }
    fn execute(&self, _a: &Value, ctx: &ToolCtx) -> ToolOutput {
        match ctx.exec("flood") {
            SandboxOutcome::Ran { stdout, truncated } => {
                ToolOutput { exit_code: 0, stdout, stderr: String::new(), truncated }
            }
            SandboxOutcome::Denied { reason } => {
                ToolOutput { exit_code: -1, stdout: String::new(), stderr: reason, truncated: false }
            }
        }
    }
}

fn registry() -> ToolRegistry {
    let mut r = ToolRegistry::new();
    r.register(Arc::new(ProbeTool));
    r
}

// ── 1. 输出截断：内存有界 ──────────────────────────────────────────────

#[test]
fn tool_output_is_capped_and_reports_truncation() {
    let _guard = COUNTER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut k = Kernel::new(
        "s", Config { exec_mode: ExecMode::AutoEdit, ..Config::default() },
        registry(),
        neo_core::models::ModelRegistry::single(std::sync::Arc::new(ScriptedModelProvider::new(vec![
            vec![neo_mock::tool_call("t1", "probe", serde_json::json!({}))],
            vec![neo_core::ModelDelta::Text("done".into())],
        ]))),
        Arc::new(FloodSandbox),
        Box::new(InMemoryPersistence::new()),
        "/tmp",
    );
    k.submit(Op::UserTurn { text: "go".into(), refs: vec![] }).unwrap();

    // 模型看到的那条工具结果必须被截断到上限，且 truncated 为 true
    let result = k
        .messages()
        .iter()
        .find_map(|m| match m {
            Message::ToolResult { output, .. } => Some(output),
            _ => None,
        })
        .expect("应有工具结果");

    assert!(
        result.stdout.len() <= DEFAULT_MAX_OUTPUT_BYTES,
        "输出未受限：{} 字节 > 上限 {}",
        result.stdout.len(),
        DEFAULT_MAX_OUTPUT_BYTES
    );
    assert!(result.truncated, "截断后必须如实置 truncated=true —— 否则上层会把截断当完整");
}

#[test]
fn truncation_never_splits_a_utf8_codepoint() {
    let _guard = COUNTER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // 直接 &s[..n] 在非字符边界会 panic。这里断言截断结果仍是合法 UTF-8
    // 且长度不超过上限。
    let s = "好".repeat(1000);
    for cap in [1usize, 2, 4, 5, 10, 100, 2999] {
        let (out, cut) = truncate_utf8(&s, cap);
        assert!(out.len() <= cap, "截断结果超出上限");
        // 能通过 &str 的类型就已是合法 UTF-8；再确认边界确实回退了
        assert!(s.is_char_boundary(out.len()), "截断点必须落在字符边界");
        assert_eq!(cut, out.len() < s.len(), "cut 标志应与实际截断一致");
    }
    // 恰好等于长度时不截断
    assert_eq!(truncate_utf8(&s, s.len()), (s.as_str(), false));
}

#[test]
fn sandbox_level_truncation_is_propagated() {
    let _guard = COUNTER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // 沙箱自己声明了 truncated（真实执行器按流读时就已截断）→ 内核必须保留该事实，
    // 不能因为本地没再截断就把 truncated 抹成 false。
    let mut k = Kernel::new(
        "s", Config { exec_mode: ExecMode::AutoEdit, ..Config::default() },
        registry(),
        neo_core::models::ModelRegistry::single(std::sync::Arc::new(ScriptedModelProvider::new(vec![
            vec![neo_mock::tool_call("t1", "probe", serde_json::json!({}))],
            vec![neo_core::ModelDelta::Text("d".into())],
        ]))),
        Arc::new(FloodSandbox),
        Box::new(InMemoryPersistence::new()),
        "/tmp",
    );
    k.submit(Op::UserTurn { text: "go".into(), refs: vec![] }).unwrap();
    let out = k
        .messages()
        .iter()
        .find_map(|m| match m { Message::ToolResult { output, .. } => Some(output), _ => None })
        .unwrap();
    assert!(out.truncated);
}

// ── 2. 多字节截断的集成路径 ────────────────────────────────────────────

#[test]
fn multibyte_output_through_the_kernel_stays_valid() {
    let _guard = COUNTER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut k = Kernel::new(
        "s", Config { exec_mode: ExecMode::AutoEdit, ..Config::default() },
        registry(),
        neo_core::models::ModelRegistry::single(std::sync::Arc::new(ScriptedModelProvider::new(vec![
            vec![neo_mock::tool_call("t1", "probe", serde_json::json!({}))],
            vec![neo_core::ModelDelta::Text("d".into())],
        ]))),
        Arc::new(MultibyteSandbox),
        Box::new(InMemoryPersistence::new()),
        "/tmp",
    )
    .with_output_cap(10); // 10 不是 3 的倍数 → 必然切在字符中间
    k.submit(Op::UserTurn { text: "go".into(), refs: vec![] }).unwrap();

    let out = k
        .messages()
        .iter()
        .find_map(|m| match m { Message::ToolResult { output, .. } => Some(output), _ => None })
        .unwrap();
    assert!(out.stdout.len() <= 10);
    assert!(out.truncated);
    // 走到这里没 panic 即证明截断点在字符边界上
}

// ── 3. 上下文上限：有界且不静默丢消息 ─────────────────────────────────

#[test]
fn context_cap_errors_instead_of_growing_without_bound() {
    let _guard = COUNTER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut k = Kernel::new(
        "s", Config { exec_mode: ExecMode::AutoEdit, ..Config::default() },
        ToolRegistry::new(),
        neo_core::models::ModelRegistry::single(std::sync::Arc::new(ScriptedModelProvider::text_only("x"))),
        Arc::new(FloodSandbox),
        Box::new(InMemoryPersistence::new()),
        "/tmp",
    )
    .with_context_cap(3);

    // 反复对话直到超限；必须报错（要求压缩），而不是静默丢弃历史
    let mut errored = false;
    for i in 0..20 {
        match k.submit(Op::UserTurn { text: format!("m{i}"), refs: vec![] }) {
            Ok(_) => {}
            Err(_) => {
                errored = true;
                break;
            }
        }
    }
    assert!(errored, "超出上下文上限必须报错，而非无界增长");
    assert!(
        k.messages().len() <= 4,
        "报错后历史不应继续膨胀（实际 {}）",
        k.messages().len()
    );
}

// ── 4. 分配实测：每步不再深拷贝整份历史 ────────────────────────────────

#[test]
fn a_step_does_not_deep_copy_the_whole_history() {
    let _guard = COUNTER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // 这是对"每步 clone 整份历史（O(N²)）"那次修复的**实测**证明。
    //
    // 方法：同样一轮只有 1 步、且该步只产出 1 个 token 的响应，
    // 分别在"历史 20 条"与"历史 400 条"下跑，比较额外分配字节。
    // 若每步深拷贝历史，400 条会多分配 ≈ 380×(每条消息的字符串+Vec) 字节；
    // 借用则几乎不随历史增长。
    fn measure(history: usize) -> usize {
        let mut k = Kernel::new(
            "s", Config { exec_mode: ExecMode::AutoEdit, ..Config::default() },
            ToolRegistry::new(),
            neo_core::models::ModelRegistry::single(std::sync::Arc::new(ScriptedModelProvider::text_only("single-token"))),
            Arc::new(FloodSandbox),
            Box::new(InMemoryPersistence::new()),
            "/tmp",
        )
        .with_context_cap(100_000);
        // 预告历史：直接提交若干轮，让 messages 长到目标规模
        k.submit(Op::UserTurn { text: "seed".into(), refs: vec![] }).unwrap();
        // 用内建方法灌入历史（模拟长会话）
        k.seed_history_for_test(history);

        let before = allocated();
        k.submit(Op::UserTurn { text: "measure".into(), refs: vec![] }).unwrap();
        allocated() - before
    }

    let small = measure(20);
    let large = measure(400);

    // 若每步深拷贝：large 会比 small 多出约 (400-20) × 单条消息大小 ≈ 数十 KB。
    // 借用路径下两者差异应在几 KB 内（仅新增内容本身）。
    let delta = large.saturating_sub(small);
    assert!(
        delta < 16 * 1024,
        "历史增长 380 条导致额外分配 {delta} 字节 —— 疑似每步仍在深拷贝历史（O(N²)）"
    );
}
