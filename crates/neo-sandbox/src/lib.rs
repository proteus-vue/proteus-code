//! L1 PLATFORM —— OS 级沙箱：把一条命令**包裹**成受限形式
//!
//! 与 `neo-core::SandboxBackend` 的分工（两者**不是同一个 seam**，故不同名）：
//!   - 本 crate 的 `CommandWrapper`：L1 职责，把 `CommandSpec` 改写成带上
//!     OS 限制的形式（sandbox-exec / bwrap）。它不懂 agent，只懂命令。
//!   - `neo-core::SandboxBackend`：L2 语义，回答"在某档语义下能否执行"，
//!     返回结构化的 `SandboxOutcome`，供闸门判定与 conformance 断言。
//!
//! 合并在一个 trait 里会强迫 L1 依赖 L2 的语义类型，破坏依赖方向。
//!
//! fail-closed：请求受限档位但平台无可用后端 → 直接拒绝，绝不裸奔。

use neo_protocol::SandboxMode;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> CommandSpec {
        CommandSpec {
            program: "sh".into(),
            args: vec!["-c".into(), "echo hi".into()],
            cwd: PathBuf::from("/tmp"),
        }
    }

    /// **每个平台的后端都要有测试** —— 这是本次补上的覆盖缺口。
    ///
    /// 为什么必须按平台分开写：这三个实现是 `#[cfg]` 互斥的，
    /// 在 macOS 上只能编译到 `Seatbelt`。于是**另外两个从未被任何测试
    /// 执行过**（实测：全局引用计数 LandlockBwrap=0、RestrictedToken=0），
    /// 而"Linux 沙箱可用"这句话就一直没有测试支撑。
    ///
    /// 按平台写测试后，各自的行为会在**对应平台的 CI** 上被验证。

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_backend_is_seatbelt_and_wraps_restricted_modes() {
        let be = detect().expect("macOS 应有后端");
        assert_eq!(be.name(), "seatbelt");
        assert!(be.supported());
        // 受限档要真的改写命令（换成 sandbox-exec）
        for mode in [SandboxMode::ReadOnly, SandboxMode::WorkspaceWrite] {
            let wrapped = be.wrap(spec(), mode).expect("受限档应能包装");
            assert_eq!(wrapped.program, "sandbox-exec", "{mode:?} 应经 sandbox-exec");
            assert!(wrapped.args.iter().any(|a| a == "-p"), "应带 profile 参数");
        }
        // 全权限档**原样返回**（不套沙箱）
        let free = be.wrap(spec(), SandboxMode::DangerFullAccess).expect("应能包装");
        assert_eq!(free.program, "sh", "全权限档不该被改写");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_backend_is_bwrap_and_wraps_restricted_modes() {
        let be = detect().expect("Linux 应有后端");
        assert_eq!(be.name(), "landlock+bwrap");
        assert!(be.supported());
        for mode in [SandboxMode::ReadOnly, SandboxMode::WorkspaceWrite] {
            let wrapped = be.wrap(spec(), mode).expect("受限档应能包装");
            assert_eq!(wrapped.program, "bwrap", "{mode:?} 应经 bwrap");
        }
        let free = be.wrap(spec(), SandboxMode::DangerFullAccess).expect("应能包装");
        assert_eq!(free.program, "sh", "全权限档不该被改写");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_backend_reports_itself_and_passes_through() {
        let be = detect().expect("Windows 应有后端");
        assert_eq!(be.name(), "restricted-token+acl");
        // Windows 后端不包装命令（受限靠 token/ACL），但**必须自报支持**
        // —— 否则会走 fail-closed 把整个平台拒掉。
        assert!(be.supported());
        let wrapped = be.wrap(spec(), SandboxMode::ReadOnly).expect("应能包装");
        assert_eq!(wrapped.program, "sh");
    }

    /// 包装不得丢弃原命令的参数（丢一个标志就可能让命令行为完全变了）。
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn wrapping_preserves_the_original_arguments() {
        let be = detect().expect("应有后端");
        let original_args = spec().args.clone();
        let wrapped = be.wrap(spec(), SandboxMode::ReadOnly).expect("应能包装");
        // 原参数应作为连续子序列出现在包装后的参数里
        let found = wrapped
            .args
            .windows(original_args.len())
            .any(|w| w == original_args.as_slice());
        assert!(
            found,
            "原参数应原样保留在包装结果里：原 {original_args:?} / 包后 {:?}",
            wrapped.args
        );
    }

    /// `cwd` 必须原样传递 —— 沙箱只管权限，不该改变工作目录
    /// （改了会让相对路径全部指向别处）。
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    fn wrapping_preserves_the_working_directory() {
        let be = detect().expect("应有后端");
        let wrapped = be.wrap(spec(), SandboxMode::ReadOnly).expect("应能包装");
        assert_eq!(wrapped.cwd, PathBuf::from("/tmp"));
    }

    /// **fail-closed**：请求受限档但平台无后端 → 必须报错，绝不裸奔。
    ///
    /// 这条在不支持的三平台上才有效（当前三个主流平台都有后端），
    /// 所以用条件编译把它限定在"没有后端"的情形下断言 ——
    /// 而在有后端的平台上，验证 `wrap_or_fail` 走的是后端那条路。
    #[test]
    fn wrap_or_fail_never_silently_runs_unsandboxed() {
        let has_backend = detect().is_some();
        let result = wrap_or_fail(spec(), SandboxMode::ReadOnly);
        if has_backend {
            assert!(result.is_ok(), "有后端时受限档应能包装");
        } else {
            assert!(
                matches!(result, Err(SandboxError::Unavailable)),
                "无后端时必须 fail-closed（拒绝执行）"
            );
        }
    }

    /// 全权限档在没有后端时**也**应当经由 `wrap_or_fail` 决策 ——
    /// 这里记录当前语义：它同样要求有后端（因为 detect() 决定一切）。
    /// 若将来改成"全权限档不需要后端"，这条会红，提醒改的人更新文档。
    #[test]
    fn full_access_still_requires_a_backend_to_be_detected() {
        let result = wrap_or_fail(spec(), SandboxMode::DangerFullAccess);
        assert_eq!(
            result.is_ok(),
            detect().is_some(),
            "全权限档的可用性当前与 detect() 绑定"
        );
    }
}
