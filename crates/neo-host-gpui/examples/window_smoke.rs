//! 冒烟：开一个真窗口，验证 NEO 的 gpui 宿主能起来、中文能渲染。
//!
//! 用 mock provider（不需要 key），不接真内核 —— 这一条只验"窗口层"。
//! 用 `NEO_GUI_SMOKE` 让它自己退出，便于脚本化验证。
use std::time::Duration;

fn main() {
    let (handle, cmd_rx, batch_tx) = neo_driver::channel();
    // 响应式宿主的唤醒信号：本冒烟不接真内核，用默认值即可
    let wake = neo_driver::WakeSignal::new();
    // 不启动真内核：用一个不消费 Op 的驱动线程占位（窗口渲染不依赖它）
    let _thread = std::thread::spawn(move || {
        for _msg in cmd_rx.iter() {
            // 什么都不做：本冒烟只验窗口与渲染
            let _ = batch_tx.send(Vec::new());
        }
    });

    // 自动退出：把「启动 + 渲染 + 采集 + 退出」合成一次运行
    let secs: u64 = std::env::var("NEO_GUI_SMOKE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if secs > 0 {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(secs));
            eprintln!("[smoke] {secs}s 到，退出");
            std::process::exit(0);
        });
    }

    let r = neo_host_gpui::run(
        handle,
        None,                       // 不接会话库（本冒烟只验窗口层）
        vec!["mock".to_string()],   // 模型名列表
        "NEO (gpui)".to_string(),
        "/tmp · default".to_string(),
        neo_protocol::ExecMode::Default,
        "mock".to_string(),
        wake,
    );
    if let Err(e) = r {
        eprintln!("[smoke] 失败：{e}");
        std::process::exit(1);
    }
}
