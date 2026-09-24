//! 子代理(Agent)的运行时:工厂 + 规格定义 + 内存持久化。
//!
//! # 子代理是什么
//!
//! 一个用 **Markdown 定义**的助手(由 `neo-agent-loader` 加载):
//! 角色正文作系统提示词、`tools` 白名单限定可用工具、可选指定模型。
//! 主内核通过 `agent_<名字>` 工具调用它 —— 子代理是一次**嵌套的内核
//! 运行**(同沙箱、同模型、精简工具集),结果作为工具输出回到主对话。
//!
//! # 安全与诚实边界
//!
//! - **沙箱是硬边界**:子内核继承主内核的沙箱档位(只读档照样拒绝写)。
//! - **无人值守**:子内核的审批策略固定为 Never —— 没有人能回答嵌套
//!   运行的提问;写操作按沙箱档位执行,这是显式取舍而非疏漏。
//! - **过程不进主转录**:子内核的对话留在内存持久化里(模型可见性
//!   按会话隔离),只有最终结果作为工具输出进入主对话与主日志。
//! - **有界**:步数上限继承主内核,防失控。

use crate::{
    Config, Kernel, ModelProvider, Tool, ToolOutput, ToolRegistry,
};
use std::sync::Arc;
use std::sync::Mutex;

/// 子代理规格(加载器产出的数据形态)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSpec {
    /// 调用名(工具名 = `agent_<name>`)
    pub name: String,
    /// 指定模型;None = 继承主内核当前模型
    pub model: Option<String>,
    /// 工具白名单。**空 = 不给任何工具**(纯文本助手是合法形态);
    /// 白名单里的未知工具在运行时报错并列出可用项。
    pub tools: Vec<String>,
    /// 系统提示词正文
    pub body: String,
}

/// 内存持久化:子代理内核的过程日志。按"模型可见即已落日志"原则,
/// 子对话也需要一份日志 —— 只是它随结果丢弃,不落盘。
#[derive(Default)]
pub struct MemoryPersistence {
    records: Mutex<Vec<crate::LoggedRecord>>,
    next: Mutex<u64>,
}

impl MemoryPersistence {
    pub fn new() -> Self { Self::default() }
}

impl crate::SessionPersistence for MemoryPersistence {
    fn append(
        &mut self,
        kind: &str,
        payload: serde_json::Value,
    ) -> Result<u64, crate::PersistenceError> {
        let mut next = self.next.lock().unwrap_or_else(|e| e.into_inner());
        *next += 1;
        let seq = *next;
        self.records
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(crate::LoggedRecord { seq, kind: kind.to_string(), payload });
        Ok(seq)
    }
    fn load(&self) -> Result<Vec<crate::LoggedRecord>, crate::PersistenceError> {
        Ok(self.records.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }
}

/// 子代理工厂:持有共享的模型/沙箱/工具全集,按白名单派生子内核。
pub struct AgentFactory {
    provider: Arc<dyn ModelProvider>,
    model_name: String,
    tools: ToolRegistry,
    sandbox: Arc<dyn crate::SandboxBackend>,
    sandbox_mode: crate::SandboxMode,
    cwd: std::path::PathBuf,
    max_steps: usize,
}

impl AgentFactory {
    pub fn new(
        provider: Arc<dyn ModelProvider>,
        model_name: String,
        tools: ToolRegistry,
        sandbox: Arc<dyn crate::SandboxBackend>,
        sandbox_mode: crate::SandboxMode,
        cwd: &std::path::Path,
        max_steps: usize,
    ) -> Self {
        Self {
            provider,
            model_name,
            tools,
            sandbox,
            sandbox_mode,
            cwd: cwd.to_path_buf(),
            max_steps,
        }
    }

    /// 运行一个子代理:按白名单裁剪工具集,系统提示词用角色正文。
    /// 返回模型最终答复(供主对话作为工具输出)。
    pub fn run(&self, spec: &AgentSpec, task: &str) -> ToolOutput {
        match self.run_inner(spec, task) {
            Ok(text) => ToolOutput { exit_code: 0, stdout: text, stderr: String::new(), truncated: false },
            Err(e) => ToolOutput { exit_code: -1, stdout: String::new(), stderr: e, truncated: false },
        }
    }

    fn run_inner(&self, spec: &AgentSpec, task: &str) -> Result<String, String> {
        // 白名单裁剪:只注册点名的工具;点名了不存在的工具 = 如实报错
        let mut sub_tools = ToolRegistry::new();
        for name in &spec.tools {
            let tool = self
                .tools
                .get(name)
                .ok_or_else(|| format!("白名单工具 {} 不存在(检查 agents 定义)", name))?;
            sub_tools.register(tool);
        }

        // 子内核:审批 Never(无人值守),沙箱硬边界继承。
        // 角色正文经 Instructions 机制进系统提示词 —— 复用同一管道,
        // 回放/落盘语义自动一致。
        let cfg = Config {
            model: self.model_name.clone(),
            sandbox_mode: self.sandbox_mode,
            approval_policy: crate::ApprovalPolicy::Never,
            ..Config::default()
        };
        let mut sub = Kernel::new(
            format!("agent-{}", spec.name),
            cfg,
            sub_tools,
            crate::models::ModelRegistry::single(self.provider.clone()),
            self.sandbox.clone(),
            Box::new(MemoryPersistence::new()),
            &self.cwd,
        )
        .with_max_steps(self.max_steps)
        .with_instructions(crate::instructions::Instructions {
            sources: vec![format!("agent:{}", spec.name)],
            block: spec.body.clone(),
            truncated: false,
        });

        let events = sub
            .submit(crate::Op::UserTurn { text: task.to_string(), refs: Vec::new() })
            .map_err(|e| format!("子代理运行失败:{e}"))?;

        // 最终答复 = 事件流里最后一条非空模型答复
        let mut final_text = String::new();
        for ev in &events {
            if let crate::EventMsg::AgentMessageDone { text } = ev {
                if !text.trim().is_empty() {
                    final_text = text.clone();
                }
            }
        }
        Ok(final_text)
    }
}

/// `agent_<名字>` 工具:主内核调用子代理的入口。
pub struct AgentTool {
    factory: Arc<AgentFactory>,
    spec: AgentSpec,
    /// 工具名（`agent_<name>`，非法字符收敛为下划线，与 MCP 同一规则）
    tool_name: String,
}

impl AgentTool {
    pub fn new(factory: Arc<AgentFactory>, spec: AgentSpec) -> Self {
        let sanitized: String = spec
            .name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
            .collect();
        Self {
            factory,
            spec,
            tool_name: format!("agent_{sanitized}"),
        }
    }
}

impl Tool for AgentTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn describe(&self) -> String {
        // 描述给模型看:职责(正文摘要)+ 可用工具白名单。字节稳定。
        let summary: String = self.spec.body.chars().take(120).collect();
        let tools = if self.spec.tools.is_empty() {
            "无工具(纯文本助手)".to_string()
        } else {
            self.spec.tools.join(", ")
        };
        format!(
            "{}(task): 委派子代理「{}」处理 task 并返回其最终答复。子代理职责:{}… 可用工具:{}。",
            self.tool_name, self.spec.name, summary, tools
        )
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {"type": "string", "description": "交给子代理的任务描述"}
            },
            "required": ["task"]
        })
    }

    /// 保守判写:子代理的白名单里可能有写工具 —— 把判定的复杂度
    /// 交给审批闸门,不在这里耍聪明。
    fn call_kind(&self, _args: &serde_json::Value) -> crate::CallKind {
        crate::CallKind::Write
    }

    fn execute(&self, args: &serde_json::Value, ctx: &crate::ToolCtx) -> ToolOutput {
        let Some(task) = args.get("task").and_then(|v| v.as_str()) else {
            return ToolOutput {
                exit_code: -1,
                stdout: String::new(),
                stderr: "缺少参数 task".into(),
                truncated: false,
            };
        };
        let mut out = self.factory.run(&self.spec, task);
        // 输出按主内核上限截断(UTF-8 安全,与其它工具同款)
        if out.stdout.len() > ctx.max_output_bytes {
            let mut end = ctx.max_output_bytes;
            while end > 0 && !out.stdout.is_char_boundary(end) {
                end -= 1;
            }
            out.stdout.truncate(end);
            out.truncated = true;
        }
        out
    }
}

/// Codex `multi_agent_v1` 命名空间下的工具套装（list / spawn）。
///
/// # 诚实边界：同步嵌套轮，不是异步 agent 生命周期
///
/// Codex 的 `wait_agent` / `interrupt_agent` / `close_agent` 假设子代理
/// 在后台以 id 运行。本内核是**单会话串行**：`AgentFactory::run` 是
/// 同步嵌套 turn，spawn 返回时任务已结束。因此这里只提供：
/// - `list_agents`：枚举已装配的子代理定义
/// - `spawn_agent`：按 `agent_type` 同步执行并返回最终答复
///
/// 异步 wait/interrupt/close **刻意不注册** —— 注册了却永远「已完成」
/// 比不注册更误导模型。
pub struct MultiAgentKit {
    factory: Arc<AgentFactory>,
    specs: Vec<AgentSpec>,
}

impl MultiAgentKit {
    pub fn new(factory: Arc<AgentFactory>, specs: Vec<AgentSpec>) -> Self {
        Self { factory, specs }
    }

    /// 把 `list_agents` + `spawn_agent` 注册进工具表（有定义才注册）。
    pub fn register(self: &Arc<Self>, reg: &mut ToolRegistry) {
        if self.specs.is_empty() {
            return;
        }
        reg.register(Arc::new(ListAgentsTool { kit: self.clone() }));
        reg.register(Arc::new(SpawnAgentTool { kit: self.clone() }));
    }

    fn find(&self, agent_type: &str) -> Option<&AgentSpec> {
        let want = agent_type.trim();
        self.specs.iter().find(|s| s.name == want || s.name.replace('-', "_") == want)
    }
}

/// `list_agents`：枚举可 spawn 的 agent_type（对齐 Codex multi_agent_v1）。
pub struct ListAgentsTool {
    kit: Arc<MultiAgentKit>,
}

impl Tool for ListAgentsTool {
    fn name(&self) -> &str {
        "list_agents"
    }

    fn describe(&self) -> String {
        "list_agents(): multi_agent_v1 — 列出可用子代理（agent_type）。返回 JSON 数组。".into()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn call_kind(&self, _args: &serde_json::Value) -> crate::CallKind {
        crate::CallKind::Read
    }

    fn execute(&self, _args: &serde_json::Value, _ctx: &crate::ToolCtx) -> ToolOutput {
        let agents: Vec<serde_json::Value> = self
            .kit
            .specs
            .iter()
            .map(|s| {
                serde_json::json!({
                    "agent_type": s.name,
                    "tool": format!("agent_{}", s.name.replace('-', "_")),
                    "tools": s.tools,
                    "summary": s.body.chars().take(80).collect::<String>(),
                })
            })
            .collect();
        ToolOutput {
            exit_code: 0,
            stdout: serde_json::to_string(&serde_json::json!({
                "namespace": "multi_agent_v1",
                "agents": agents,
            }))
            .unwrap_or_else(|_| "[]".into()),
            stderr: String::new(),
            truncated: false,
        }
    }
}

/// `spawn_agent`：按 agent_type 同步跑一轮子代理（Codex multi_agent_v1）。
pub struct SpawnAgentTool {
    kit: Arc<MultiAgentKit>,
}

impl Tool for SpawnAgentTool {
    fn name(&self) -> &str {
        "spawn_agent"
    }

    fn describe(&self) -> String {
        "spawn_agent(agent_type, task): multi_agent_v1 — 同步委派子代理并返回最终答复。\
         先 list_agents 看可用类型。本实现非异步：返回时任务已结束。"
            .into()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "agent_type": {
                    "type": "string",
                    "description": "子代理类型（list_agents 里的 agent_type）"
                },
                "task": {
                    "type": "string",
                    "description": "交给子代理的任务描述"
                }
            },
            "required": ["agent_type", "task"]
        })
    }

    fn call_kind(&self, _args: &serde_json::Value) -> crate::CallKind {
        crate::CallKind::Write
    }

    fn execute(&self, args: &serde_json::Value, ctx: &crate::ToolCtx) -> ToolOutput {
        let agent_type = args
            .get("agent_type")
            .or_else(|| args.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let task = args.get("task").and_then(|v| v.as_str()).unwrap_or("");
        if agent_type.trim().is_empty() || task.trim().is_empty() {
            return ToolOutput {
                exit_code: -1,
                stdout: String::new(),
                stderr: "spawn_agent 需要 agent_type 与 task".into(),
                truncated: false,
            };
        }
        let Some(spec) = self.kit.find(agent_type) else {
            let available: Vec<&str> = self.kit.specs.iter().map(|s| s.name.as_str()).collect();
            return ToolOutput {
                exit_code: -1,
                stdout: String::new(),
                stderr: format!("未知 agent_type「{agent_type}」。可用：{available:?}"),
                truncated: false,
            };
        };
        let mut out = self.kit.factory.run(spec, task);
        if out.stdout.len() > ctx.max_output_bytes {
            let mut end = ctx.max_output_bytes;
            while end > 0 && !out.stdout.is_char_boundary(end) {
                end -= 1;
            }
            out.stdout.truncate(end);
            out.truncated = true;
        }
        out
    }
}
