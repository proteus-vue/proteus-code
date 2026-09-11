# L1 · 平台层规格

**crates**：`neo-sandbox`、`neo-platform`
**依赖**：`neo-protocol`（L0，共享枚举如 SandboxMode）
**职责**：OS 原语封装，无任何业务语义

## neo-sandbox

```rust
pub enum SandboxMode { ReadOnly, WorkspaceWrite, DangerFullAccess }

pub trait SandboxBackend {
    fn supported(&self) -> bool;
    fn wrap(&self, cmd: &CommandSpec, mode: SandboxMode) -> Result<WrappedCommand>;
}

// 三平台实现
#[cfg(target_os = "macos")]  pub struct SeatbeltBackend;    // sandbox-exec -p
#[cfg(target_os = "linux")]   pub struct LandlockBwrap;     // Landlock + bubblewrap + seccomp
#[cfg(target_os = "windows")] pub struct RestrictedToken;   // restricted token + ACL + WFP
```

### fail-closed（硬要求，继承自 DSH）
```
请求 ReadOnly/WorkspaceWrite 但平台无可用后端
        → 返回 Err(SANDBOX_UNAVAILABLE)，拒绝执行
        → 绝不降级为"无沙箱执行"
```

### 网络策略
`WorkspaceWrite` 下**默认断网**。开启方式：
```toml
[sandbox_workspace_write]
network_access = true
writable_roots = ["/extra/path"]
```

### 平台差异说明（必须写进文档，不可回避）
| 平台 | 机制 | 已知限制 |
|---|---|---|
| macOS | Seatbelt | 读取/网络限制完整 |
| Linux | Landlock + bwrap + seccomp | 需 Linux 5.13+；需非特权 userns |
| Windows | restricted token + ACL | **读取、网络、进程可见性不受限**（诚实标注） |
| WSL2 | 走 Linux 实现 | WSL1 不支持（bwrap 依赖） |

## neo-platform

| 能力 | 说明 |
|---|---|
| `process_hardening` | pre-main 反调试、反转储、环境清理 |
| `fs_notify` | inotify / FSEvents / ReadDirectoryChangesW（替代 Node fs.watch） |
| `git` | 用 git2 原生 worktree 操作（子 agent 隔离用） |
| `argv0_dispatch` | 单二进制多入口（对齐 Codex `codex-arg0`） |

## 验收
- [ ] 三平台沙箱各自可阻止越界写入（用真实越权命令测试）
- [ ] 无可用后端时返回 `SANDBOX_UNAVAILABLE` 而非裸奔
- [ ] `neo-platform` 不依赖 L2 及以上（架构守卫校验）
