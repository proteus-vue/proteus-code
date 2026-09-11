//! L5 HOST · Exec（无头 / CI）
//! 零交互、可脚本化、可管道化。
pub fn run(prompt: &str) -> String { format!("[exec] {prompt}") }
