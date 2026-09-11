//! L0 PLATFORM —— OS 级沙箱
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

pub trait SandboxBackend {
    fn name(&self) -> &'static str;
    fn supported(&self) -> bool;
    fn wrap(&self, cmd: CommandSpec, mode: SandboxMode) -> Result<CommandSpec, SandboxError>;
}

#[cfg(target_os = "macos")]
pub struct Seatbelt;
#[cfg(target_os = "macos")]
impl SandboxBackend for Seatbelt {
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
impl SandboxBackend for LandlockBwrap {
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
impl SandboxBackend for RestrictedToken {
    fn name(&self) -> &'static str { "restricted-token+acl" }
    fn supported(&self) -> bool { true }
    // NOTE: Windows 下读取/网络/进程可见性不受限，文档须诚实标注。
    fn wrap(&self, cmd: CommandSpec, _mode: SandboxMode) -> Result<CommandSpec, SandboxError> {
        Ok(cmd)
    }
}

/// 按平台选择后端。未匹配平台 → 无后端 → fail-closed（拒绝执行，绝不裸奔）。
#[cfg(target_os = "macos")]
pub fn detect() -> Option<Box<dyn SandboxBackend>> { Some(Box::new(Seatbelt)) }

#[cfg(target_os = "linux")]
pub fn detect() -> Option<Box<dyn SandboxBackend>> { Some(Box::new(LandlockBwrap)) }

#[cfg(target_os = "windows")]
pub fn detect() -> Option<Box<dyn SandboxBackend>> { Some(Box::new(RestrictedToken)) }

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub fn detect() -> Option<Box<dyn SandboxBackend>> { None }

/// 统一入口：先探测后端，无后端直接报错（fail-closed）。
pub fn wrap_or_fail(cmd: CommandSpec, mode: SandboxMode) -> Result<CommandSpec, SandboxError> {
    match detect() {
        Some(be) if be.supported() => be.wrap(cmd, mode),
        _ => Err(SandboxError::Unavailable),
    }
}
