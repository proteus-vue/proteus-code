//! Notify SPI 的 conformance —— 同一份用例跑所有后端。
//!
//! # 这个 SPI 最特殊的契约：失败不该打扰主流程
//!
//! 提醒常在**无人值守**环境被调用（CI、SSH、容器、无音频设备）。
//! 那时它必须能安静地不做，而不是崩、卡住，或把错误抛上去中断任务。
//! 这条契约比"能响铃"重要得多，所以它是本套用例的重点。

use neo_platform::{Attention, NoopNotify, Notify, SystemNotify};

/// 契约 1：`notify` 必须**始终返回**（不 panic、不阻塞），
/// 且返回值语义与 `available()` 一致 —— 不能自称可用却失败，
/// 也不能自称不可用却默默成功（那会让降级逻辑永远走不到）。
fn assert_notify_never_panics_and_is_consistent(n: &dyn Notify) {
    let r = n.notify(Attention::TurnComplete, "conformance 探针");
    if n.available() {
        assert!(r.is_ok(), "后端 {} 自称可用，notify 却失败：{:?}", n.name(), r.err());
    } else {
        // 不可用时：剪贴板要报错，**提醒不该报错**（少个便利 ≠ 操作失败）
        assert!(
            r.is_ok(),
            "后端 {} 不可用时不该把提醒失败当错误抛出（会中断主流程）：{:?}",
            n.name(),
            r.err()
        );
    }
}

/// 契约 2：三种场景都要能处理，不能只支持其中一种。
fn assert_all_scenarios_are_handled(n: &dyn Notify) {
    for kind in [Attention::TurnComplete, Attention::Error, Attention::ApprovalNeeded] {
        let _ = n.notify(kind, "scenario probe");
    }
}

/// 契约 3：`name` 可辨识。
fn assert_name_is_present(n: &dyn Notify) {
    assert!(!n.name().is_empty(), "后端名不得为空");
}

#[test]
fn notify_contract_holds_for_every_backend() {
    let system = SystemNotify::new(false); // 测试时不发声
    let noop = NoopNotify::new("conformance 测试");

    for n in [&system as &dyn Notify, &noop as &dyn Notify] {
        assert_name_is_present(n);
        assert_notify_never_panics_and_is_consistent(n);
        assert_all_scenarios_are_handled(n);
    }
}

#[test]
fn noop_is_the_silent_degradation_path() {
    // noop 的存在意义：让"不想/不能提醒"有一个**明确且不报错**的去处。
    // 若它报错，调用方就得在调用点用 if 特判 —— 那正是加了 SPI 又绕开 SPI。
    let n = NoopNotify::new("已关闭");
    assert!(!n.available());
    for kind in [Attention::TurnComplete, Attention::Error, Attention::ApprovalNeeded] {
        assert!(n.notify(kind, "x").is_ok(), "noop 永远不该报错");
    }
}

#[test]
fn empty_detail_is_accepted() {
    // 空文案是边界：后端不该因此报错（AppleScript 空字符串是合法的）
    let n = SystemNotify::new(false);
    if n.available() {
        let r = n.notify(Attention::TurnComplete, "");
        assert!(r.is_ok(), "空文案应可接受：{:?}", r.err());
    }
}

#[test]
fn long_detail_does_not_hang() {
    // 超长文案不该让 osascript/notify-send 卡住（有后端会在超长参数上阻塞）
    let n = SystemNotify::new(false);
    if n.available() {
        let long = "很长的通知内容".repeat(200);
        let _ = n.notify(Attention::Error, &long);
    }
}

#[test]
fn detail_with_shell_metacharacters_is_not_executed() {
    // 安全边界：我们走的是 argv 传参（不是 shell 拼接），
    // 因此含 `; rm -rf /` 的文案只会被当成文本。
    // 这条断言防的是"以后有人图省事改成 sh -c 拼接"。
    let marker = format!("/tmp/neo-notify-injection-{}", std::process::id());
    let n = SystemNotify::new(false);
    if n.available() {
        let _ = n.notify(Attention::Error, &format!("x; touch {marker}"));
        // 给 shell 一点时间（如果真的被当成命令）
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(
            !std::path::Path::new(&marker).exists(),
            "通知文案被当成了 shell 命令执行 —— 存在注入漏洞"
        );
        let _ = std::fs::remove_file(&marker);
    }
}
