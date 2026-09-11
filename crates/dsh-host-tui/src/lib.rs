//! L5 HOST · TUI（ratatui + crossterm）
//!
//! 铁律：不含业务逻辑。只做渲染、输入解析、消费 EventMsg。

/// 快捷键（对齐 Codex）
pub enum Key {
    ClearScreen,      // Ctrl+L
    CopyLastOutput,   // Ctrl+O
    SearchHistory,    // Ctrl+R
    ExternalEditor,   // Ctrl+G
    QueueNextInput,   // Tab（运行中排队，不打断）
    EditLastMessage,  // Esc Esc
    CycleExecMode,    // Shift+Tab（ZCode 五档）
    FuzzyFileSearch,  // @
}

/// Slash 命令
pub const SLASH_COMMANDS: &[&str] = &[
    "/goal", "/review", "/model", "/fork", "/permissions",
    "/compact", "/diff", "/status", "/agent", "/debug-config",
];

pub fn render(event_json: &str) -> String { format!("[tui] {event_json}") }
