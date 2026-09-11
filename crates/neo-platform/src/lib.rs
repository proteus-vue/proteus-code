//! L0 PLATFORM —— 进程加固、文件监听、git worktree
pub fn harden_process() { /* pre-main 反调试 / 反转储 / 环境变量清理 */ }
pub fn watch(_root: &str) { /* inotify / FSEvents / ReadDirectoryChangesW */ }
pub fn worktree_path(root: &str, name: &str) -> String { format!("{root}/.neo/worktrees/{name}") }
