//! 窗口壳：把系统 webview 指向 Web 宿主服务的页面。
//!
//! # 设计：窗口只是壳，不是第二个宿主
//!
//! 桌面窗口**不**引入新的界面代码或 IPC 层 —— 它内嵌一个系统 webview
//! （macOS: WKWebView / Windows: WebView2 / Linux: WebKitGTK），指向
//! Web 宿主在本地回环端口上服务的同一个页面。收益：
//!
//! - **T6 宿主等价天然成立**：桌面跑的就是 Web 宿主，不存在"第三套
//!   事件消费逻辑"要证明等价；
//! - **零重复界面**：任务输入、SSE 事件流、审批、目标栏全部来自
//!   `neo-host-web` 的单文件页面；
//! - 恶意网页面的风险面不变：webview 只访问 `127.0.0.1` 的本地服务
//!   （与用户自己开浏览器访问一致）。
//!
//! # 有意的取舍
//!
//! - 每请求 `Connection: close`、webview 与 HTTP 服务之间无身份握手 ——
//!   端口只绑 `127.0.0.1`，与 `neo serve` 的安全模型一致（Web 宿主
//!   对外监听需先加鉴权，见诚实清单）。
//! - 窗口关闭 = 退出应用：内核随 op 通道关闭而停机，不做托盘/后台驻留。

use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::window::WindowBuilder;

/// 打开一个系统 webview 窗口指向 `url`，**阻塞**直到窗口关闭。
pub fn run_window(url: &str, title: &str) -> Result<(), String> {
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title(title)
        .build(&event_loop)
        .map_err(|e| format!("创建窗口失败：{e}"))?;
    let webview = wry::WebViewBuilder::new()
        .with_url(url.to_string())
        .build(&window)
        .map_err(|e| format!("创建 webview 失败：{e}"))?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            Event::MainEventsCleared => window.request_redraw(),
            _ => {}
        }
        // webview 必须活到事件循环结束（macOS 上视图由窗口持有，
        // 但 drop 顺序不确定 —— 显式延长生命周期）
        let _ = &webview;
    });
}
