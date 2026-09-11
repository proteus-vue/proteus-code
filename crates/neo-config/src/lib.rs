//! 四级配置：内置 -> /etc -> 用户 -> 项目 -> CLI flag
//! 安全敏感键只允许在用户级设置，项目级忽略。
use neo_protocol::{ApprovalPolicy, ExecMode, SandboxMode};
use serde::Deserialize;

pub const AGENTS_MAX_BYTES: usize = 32 * 1024; // 32 KiB

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub model: String,
    pub exec_mode: ExecMode,
    pub sandbox_mode: SandboxMode,
    pub approval_policy: ApprovalPolicy,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: "deepseek-v4-flash".into(),
            exec_mode: ExecMode::Default,
            sandbox_mode: SandboxMode::WorkspaceWrite,
            approval_policy: ApprovalPolicy::OnRequest,
        }
    }
}

/// 协议层的 `SessionPatch` → 配置层的 `ConfigPatch`。
///
/// 为什么要这道转换而不是直接用一个类型：**协议层是线协议，配置层是本地策略**。
/// 两者合并会让线协议随配置项演进而频繁变更版本 —— 那正是 T2（协议确定性）
/// 最怕的。转换函数把"wire 能改什么"与"本地能配什么"显式分开。
impl From<neo_protocol::SessionPatch> for ConfigPatch {
    fn from(p: neo_protocol::SessionPatch) -> Self {
        Self {
            model: p.model,
            exec_mode: p.exec_mode,
            sandbox_mode: p.sandbox_mode,
            approval_policy: p.approval_policy,
        }
    }
}

/// 后置层覆盖前置层
pub fn merge(base: Config, over: ConfigPatch) -> Config {
    Config {
        model: over.model.unwrap_or(base.model),
        exec_mode: over.exec_mode.unwrap_or(base.exec_mode),
        sandbox_mode: over.sandbox_mode.unwrap_or(base.sandbox_mode),
        approval_policy: over.approval_policy.unwrap_or(base.approval_policy),
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConfigPatch {
    pub model: Option<String>,
    pub exec_mode: Option<ExecMode>,
    pub sandbox_mode: Option<SandboxMode>,
    pub approval_policy: Option<ApprovalPolicy>,
}

/// 执行模式解析结果。
///
/// 注意第三维 `file_edit`：ZCode 的 Default 与 AutoEdit 在"沙箱 × 审批"双轴上
/// 完全相同，真实差异在**工具类别粒度** —— 文件编辑是否自动放行。
/// 缺了这一维，两个模式在底层不可区分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeResolution {
    pub sandbox: SandboxMode,
    pub approval: ApprovalPolicy,
    pub file_edit: FileEditPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileEditPolicy { Auto, Ask }

pub fn resolve(mode: ExecMode) -> ModeResolution {
    use ApprovalPolicy as A;
    use ExecMode as E;
    use FileEditPolicy as F;
    use SandboxMode as S;
    match mode {
        E::Plan          => ModeResolution { sandbox: S::ReadOnly,         approval: A::OnRequest, file_edit: F::Ask  },
        E::ConfirmBefore => ModeResolution { sandbox: S::WorkspaceWrite,   approval: A::Untrusted, file_edit: F::Ask  },
        E::Default       => ModeResolution { sandbox: S::WorkspaceWrite,   approval: A::OnRequest, file_edit: F::Ask  },
        E::AutoEdit      => ModeResolution { sandbox: S::WorkspaceWrite,   approval: A::OnRequest, file_edit: F::Auto },
        E::FullAccess    => ModeResolution { sandbox: S::DangerFullAccess, approval: A::Never,     file_edit: F::Auto },
    }
}
