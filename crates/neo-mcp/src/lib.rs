//! MCP（Model Context Protocol）客户端 —— 外部工具服务器的接入层。
//!
//! # 它做什么
//!
//! 把一个 MCP 服务器（stdio 传输，JSON-RPC 2.0，**换行分隔**的 JSON 消息）
//! 暴露的工具变成 [`neo_core::Tool`]，从而复用内核现成的整条链路：
//! 提示词注入、语义分类、审批闸门、输出上限、会话落盘。外部工具不需要
//! 任何特殊通道 —— 对内核来说它就是普通工具。
//!
//! # 为什么不新增 SPI
//!
//! MCP 不是新 seam：工具的 seam 早就是 [`neo_core::Tool`]，MCP 只是这个
//! seam 的**另一个实现来源**（如同 `neo-capability` 提供内置工具）。
//! 再立一个"MCP 后端"SPI 会把同一个事实记两遍 —— 那是账本漂移的温床。
//!
//! # 一个必须说清的安全边界：为什么 `execute` 不走 `ctx.exec`
//!
//! 沙箱管辖的是**工具在本地系统发起的命令执行**（bash 的每条命令）。
//! MCP 工具的执行是「向用户配置的外部服务器进程的 stdin 写一条 JSON-RPC」——
//! 这个进程是**装配时**由用户级配置启动的常驻服务，与模型 provider 的
//! HTTP 端点同类：它是 harness 的集成对象，不是工具发起的本地进程。
//! 因此这里没有、也不应有绕过沙箱执行本地命令的入口 —— 工具没有任何
//! `Command::new`；服务器进程本身由配置文件声明（见 [`config`]）。
//!
//! # 配置与密钥同一套纪律：只允许用户级
//!
//! 项目级 `mcp.json` **显式拒绝**（不是静默忽略）：一个仓库可以指定的
//! 可执行文件就是供应链注入 —— `git clone` 下来跑一次就执行了任意程序。
//! 用户级配置是用户自己的决定，项目级是仓库作者的决定，两者必须分清。
//!
//! # 有界性
//!
//! 所有等待都有超时（握手/列工具/调用各自独立），stderr 用固定容量的
//! 环形缓冲保存尾部（服务器可能刷屏，无界读就是内存漏洞），
//! 工具输出按内核给的字节上限截断（UTF-8 安全）。
pub mod client;
pub mod config;
pub mod tool;
pub mod wire;

pub use client::{McpClient, McpError, ServerSpec};
pub use config::load_user_config;
pub use tool::McpTool;

/// 启动一个服务器、完成握手、列出工具，返回（共享连接，工具集合）。
///
/// 装配点（`build_kernel`）用这一步把外部工具接进 ToolRegistry；
/// 全部工具共享同一条连接 —— 一个服务器进程服务它的全部工具。
pub fn connect(
    spec: &ServerSpec,
) -> Result<(std::sync::Arc<std::sync::Mutex<McpClient>>, Vec<McpTool>), McpError> {
    let client = McpClient::spawn(spec)?;
    let conn = std::sync::Arc::new(std::sync::Mutex::new(client));
    let infos = conn.lock().unwrap_or_else(|e| e.into_inner()).list_tools()?;
    let tools = infos
        .iter()
        .map(|info| McpTool::new(&spec.name, info, conn.clone()))
        .collect();
    Ok((conn, tools))
}
