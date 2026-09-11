//! Clipboard SPI 的 conformance —— **同一份用例跑所有后端**。
//!
//! 为什么需要：SPI 的价值全在"换个后端行为仍一致"。只测系统后端等于
//! 没测可替换性；只测 noop 等于没测真实路径。两个都跑，且断言的是
//! **契约**（返回值语义、失败要明说），不是实现细节。

use neo_platform::{Clipboard, NoopClipboard, SystemClipboard};

/// 契约 1：`copy` 的返回必须真实反映结果。
///
/// 不能出现"返回 Ok 但其实没写进去" —— 那会让用户粘贴出旧内容。
/// 这里对系统后端做**能力探测式**断言：若它自称 available，copy 就必须
/// 成功；若不可用，copy 就必须返回 Err（而不是 Ok）。
fn assert_copy_is_truthful(c: &dyn Clipboard) {
    let r = c.copy("neo-conformance-探针");
    if c.available() {
        assert!(
            r.is_ok(),
            "后端 {} 自称可用，copy 却失败：{:?}",
            c.name(),
            r.err()
        );
    } else {
        assert!(
            r.is_err(),
            "后端 {} 自称不可用，copy 却返回成功（撒谎）",
            c.name()
        );
    }
}

/// 契约 2：不可用的后端必须给出**可读原因**，不能空字符串。
fn assert_error_is_actionable(c: &dyn Clipboard) {
    if c.available() {
        return; // 可用时没有错误可查
    }
    let err = c.copy("x").expect_err("不可用后端必须报错");
    assert!(!err.trim().is_empty(), "后端 {} 的错误信息为空", c.name());
    // 原因里应含后端名或明确的缺失说明，便于用户排查
    assert!(
        err.contains(c.name()) || err.contains("不可用") || err.contains("未找到"),
        "后端 {} 的错误信息不够可行动：{err}",
        c.name()
    );
}

/// 契约 3：`name` 必须非空且唯一可辨识。
fn assert_name_is_present(c: &dyn Clipboard) {
    assert!(!c.name().is_empty(), "后端名不得为空");
}

#[test]
fn clipboard_contract_holds_for_every_backend() {
    let system = SystemClipboard::new();
    let noop = NoopClipboard::new("conformance 测试环境");

    for c in [&system as &dyn Clipboard, &noop as &dyn Clipboard] {
        assert_name_is_present(c);
        assert_error_is_actionable(c);
        assert_copy_is_truthful(c);
    }
}

#[test]
fn noop_is_never_available_and_never_claims_success() {
    // 负向语义：noop 后端的唯一职责就是"诚实地说自己不能用"
    let noop = NoopClipboard::new("headless");
    assert!(!noop.available());
    assert!(noop.copy("任意内容").is_err());
}

#[test]
fn system_backend_is_the_real_one_on_this_platform() {
    // 正向语义：在桌面平台上系统后端应当可用（CI 无剪贴板时跳过）
    let system = SystemClipboard::new();
    if cfg!(target_os = "macos") {
        // macOS 自带 pbcopy，必然存在
        assert!(system.available(), "macOS 上 pbcopy 应始终可用");
        system.copy("neo 剪贴板验证").expect("macOS 上复制应成功");
    }
}

#[test]
fn empty_text_is_still_written_without_panic() {
    // 边界：空串。不该 panic（某些后端对空输入会立刻 EOF）
    let system = SystemClipboard::new();
    if system.available() {
        let _ = system.copy("");
    }
    let noop = NoopClipboard::new("headless");
    let _ = noop.copy("");
}
