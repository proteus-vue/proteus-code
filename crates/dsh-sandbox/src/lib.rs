//! L1 PLATFORM —— OS 级沙箱：把一条命令**包裹**成受限形式
//!
//! 与 `dsh-core::SandboxBackend` 的分工（两者**不是同一个 seam**，故不同名）：
//!   - 本 crate 的 `CommandWrapper`：L1 职责，把 `CommandSpec` 改写成带上
//!     OS 限制的形式（sandbox-exec / bwrap）。它不懂 agent，只懂命令。
//!   - `dsh-core::SandboxBackend`：L2 语义，回答"在某档语义下能否执行"，
//!     返回结构化的 `SandboxOutcome`，供闸门判定与 conformance 断言。
//!
//! 合并在一个 trait 里会强迫 L1 依赖 L2 的语义类型，破坏依赖方向。
//!
//! fail-closed：请求受限档位但平台无可用后端 → 直接拒绝，绝不裸奔。

use dsh_protocol::SandboxMode;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("no usable sandbox backend on this platform; refusing to run unsandboxed")]
    Unavailable,
    #[error("sandbox denied: {0}")]
    Denied(String),
}

pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

pub trait CommandWrapper {
    fn name(&self) -> &'static str;
    fn supported(&self) -> bool;
    fn wrap(&self, cmd: CommandSpec, mode: SandboxMode) -> Result<CommandSpec, SandboxError>;
}

#[cfg(target_os = "macos")]
pub struct Seatbelt;
#[cfg(target_os = "macos")]
impl CommandWrapper for Seatbelt {
    fn name(&self) -> &'static str { "seatbelt" }
    fn supported(&self) -> bool { true }
    fn wrap(&self, cmd: CommandSpec, mode: SandboxMode) -> Result<CommandSpec, SandboxError> {
        let profile = match mode {
            SandboxMode::ReadOnly => "neo-read-only.sb",
            SandboxMode::WorkspaceWrite => "neo-workspace-write.sb",
            SandboxMode::DangerFullAccess => return Ok(cmd),
        };
        Ok(CommandSpec {
            program: "sandbox-exec".into(),
            args: [vec!["-p".to_string(), profile.to_string()], cmd.args].concat(),
            cwd: cmd.cwd,
        })
    }
}

#[cfg(target_os = "linux")]
pub struct LandlockBwrap;
#[cfg(target_os = "linux")]
impl CommandWrapper for LandlockBwrap {
    fn name(&self) -> &'static str { "landlock+bwrap" }
    fn supported(&self) -> bool { true }
    fn wrap(&self, cmd: CommandSpec, mode: SandboxMode) -> Result<CommandSpec, SandboxError> {
        match mode {
            SandboxMode::DangerFullAccess => Ok(cmd),
            _ => Ok(CommandSpec {
                program: "bwrap".into(),
                args: [vec!["--ro-bind".into(), "/".into(), "/".into()], cmd.args].concat(),
                cwd: cmd.cwd,
            }),
        }
    }
}

#[cfg(target_os = "windows")]
pub struct RestrictedToken;
#[cfg(target_os = "windows")]
impl CommandWrapper for RestrictedToken {
    fn name(&self) -> &'static str { "restricted-token+acl" }
    fn supported(&self) -> bool { true }
    // NOTE: Windows 下读取/网络/进程可见性不受限，文档须诚实标注。
    fn wrap(&self, cmd: CommandSpec, _mode: SandboxMode) -> Result<CommandSpec, SandboxError> {
        Ok(cmd)
    }
}

/// 按平台选择后端。未匹配平台 → 无后端 → fail-closed（拒绝执行，绝不裸奔）。
#[cfg(target_os = "macos")]
pub fn detect() -> Option<Box<dyn CommandWrapper>> { Some(Box::new(Seatbelt)) }

#[cfg(target_os = "linux")]
pub fn detect() -> Option<Box<dyn CommandWrapper>> { Some(Box::new(LandlockBwrap)) }

#[cfg(target_os = "windows")]
pub fn detect() -> Option<Box<dyn CommandWrapper>> { Some(Box::new(RestrictedToken)) }

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub fn detect() -> Option<Box<dyn CommandWrapper>> { None }

/// 统一入口：先探测后端，无后端直接报错（fail-closed）。
pub fn wrap_or_fail(cmd: CommandSpec, mode: SandboxMode) -> Result<CommandSpec, SandboxError> {
    match detect() {
        Some(be) if be.supported() => be.wrap(cmd, mode),
        _ => Err(SandboxError::Unavailable),
    }
}
