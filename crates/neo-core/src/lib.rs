//! L2 CORE —— 内核（固定语义核心，见 ADR-0001 与方法论文档）
//!
//! # 本 crate 的三条铁律
//!
//! 1. **只认契据，不认实现。** 模型 / 沙箱 / 持久化 / 工具全部是 trait，
//!    具体实现由上层注入 —— 这是「L2 不依赖 L3/L4/L5」的技术保证。
//! 2. **沙箱是内核的保证，不是各工具的自觉。** `Tool::execute` 必须接收带沙箱的
//!    [`ToolCtx`]；工具想绕过沙箱也没有入口。安全边界不能靠约定。
//! 3. **模型可见即已落日志。** 凡进入模型请求的内容都必须能从会话日志重建，
//!    否则回放（T2）与审计失去意义。
//!
//! # 本轮实现覆盖
//!
//! - turn/step 主循环（一步 = 一次模型请求 + 其工具调用；一轮 = 0..N 步）
//! - 审批挂起与恢复（`Op::Approve` 回到同一步的同一位置继续）
//! - 沙箱（硬边界）× 审批（流程）× 文件编辑粒度 的三维闸门
//! - 事件发射（`EventMsg`）与 append-only 落盘
//! - 确定性：审批 id 由 (turn, step, index) 派生，事件内容不含时钟/随机

use neo_config::{resolve, Config, FileEditPolicy, ModeResolution};
use neo_protocol::*;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// 一步内允许的最大工具调用数（防御失控模型）。
pub const MAX_TOOL_CALLS_PER_STEP: usize = 32;
/// 一轮内允许的最大步数。超出即报错结束该轮，不无限循环。
pub const DEFAULT_MAX_STEPS: usize = 8;

/// 上下文消息上限。
///
/// 达到上限时内核**报错并要求压缩**，而不是静默丢消息 ——
/// 静默丢会让模型的视角与日志不一致，破坏"模型可见即已落盘"的铁律。
/// 压缩本身是 L4 orchestration 的职责（`Compact` Op），当前**未实现**。
pub const DEFAULT_MAX_CONTEXT_MESSAGES: usize = 4096;

// ══════════════════════════════════════════════════════════════════════
// 会话消息：模型可见的内容
// ══════════════════════════════════════════════════════════════════════

/// 一次工具调用请求（模型产出）。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolInvocation {
    pub id: ToolCallId,
    pub name: String,
    pub arguments: Value,
}

/// 模型可见的一条消息。
///
/// 这是**派生物**：真实来源是 append-only 会话日志。保留结构化形态是为了
/// 组装请求时不必反复解析日志。
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    System(String),
    User(String),
    Assistant { text: String, tool_calls: Vec<ToolInvocation> },
    ToolResult { id: ToolCallId, name: String, output: ToolOutput },
}

// ══════════════════════════════════════════════════════════════════════
// 扩展点 1/4：ModelProvider（流式 + 支持工具调用）
// ══════════════════════════════════════════════════════════════════════

/// 工具对模型的呈现（只含模型需要知道的字段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
}

/// 一次模型请求。
///
/// `messages` 是**借用**的切片而非拥有所有权的 `Vec`：
/// 若每步都 `clone()` 整份历史，一轮 N 步就是 O(N²) 拷贝 —— 长会话下这是
/// 主要热点。真实 provider 本来就会先把请求序列化出去再流式读回，
/// 不需要持有历史，所以借用不构成限制。
#[derive(Debug)]
pub struct ModelRequest<'a> {
    pub system: &'a str,
    pub messages: &'a [Message],
    pub tools: &'a [ToolSchema],
}

/// 模型响应的增量。顺序即语义：文本增量 → 工具调用 → 用量。
#[derive(Debug, Clone, PartialEq)]
pub enum ModelDelta {
    Text(String),
    ToolCall(ToolInvocation),
    Usage { input_tokens: u64, output_tokens: u64 },
}

/// 一次模型响应的增量序列。
///
/// 用迭代器而非 `Vec` 是为了**保留流式语义**：真实 provider 边收边 yield，
/// 内核也边收边发 `AgentMessageDelta`。换成真实流式实现时内核循环不变。
pub type ModelStream = Box<dyn Iterator<Item = ModelDelta> + Send>;

/// 扩展点：换模型。
///
/// **为什么不是 `complete(prompt) -> String`**：那个签名无法表达工具调用，
/// 而工具调用是 agent 的本质 —— 按 SPI-First 这是 AP-05
/// （接口表达不了所需行为的抽象是假 SPI）。
pub trait ModelProvider: Send + Sync {
    fn name(&self) -> &str;
    /// 产生一次模型响应的增量序列。**必须确定性**：同请求同序列（T2 可回放的前提）。
    ///
    /// 返回的流不借用 `request`：实现应在内部把需要的内容序列化/复制出去
    /// （真实 provider 一次 HTTP 请求即如此），从而让调用方可以立即释放借用。
    fn stream(&self, request: &ModelRequest<'_>) -> ModelStream;
}

// ══════════════════════════════════════════════════════════════════════
// 扩展点 2/4：Tool
// ══════════════════════════════════════════════════════════════════════

/// 工具调用的语义类别 —— 闸门据此判定，而不是按工具名字符串猜。
///
/// 旧版用 `if call == "bash"` 判断，脆弱：新增工具就漏判、改名就失效。
/// 类别是**工具自己的知识**，应由工具声明。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKind {
    /// 只读（不改变本地状态）
    Read,
    /// 写本地状态（含文件变更）
    Write,
    /// 访问网络
    Network,
    /// 与人交互（提问等）
    Interactive,
}

/// 工具执行上下文：内核把**沙箱与工作目录**注入进来。
///
/// 存在的意义：让「沙箱是硬边界」成为内核的**结构保证** ——
/// 工具无法自行选择沙箱模式，也没有不经沙箱执行命令的入口。
/// 单次工具调用允许回灌到内存的输出上限（字节）。
///
/// 为什么必须有：Rust 保证内存**安全**，但不保证内存**有界** ——
/// 一条 `yes` 或 `find /` 能把进程撑爆。上限是内核的义务，不是工具的自觉。
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 256 * 1024;

pub struct ToolCtx<'a> {
    pub sandbox: &'a dyn SandboxBackend,
    pub mode: SandboxMode,
    pub cwd: &'a std::path::Path,
    /// 单次调用的输出上限；超出即截断并置 `truncated`。
    pub max_output_bytes: usize,
}

impl ToolCtx<'_> {
    /// 经沙箱执行一条命令。**工具执行命令的唯一入口。**
    ///
    /// 上限传给实现：真实执行器必须**边读边限**（读取时就停止累积）。
    /// 上限也仍是内核的强制边界：实现即便超限返回，此处再兜一次（纵深防御），
    /// 保证无论实现多粗心，进到模型上下文的内容都不会无界。
    pub fn exec(&self, command: &str) -> SandboxOutcome {
        let outcome = self.sandbox.execute(self.mode, command, self.max_output_bytes);
        match outcome {
            SandboxOutcome::Ran { stdout, truncated } => {
                let (text, cut) = truncate_utf8(&stdout, self.max_output_bytes);
                SandboxOutcome::Ran { stdout: text.to_string(), truncated: truncated || cut }
            }
            denied => denied,
        }
    }

    /// 经沙箱写文件。**工具修改文件的唯一入口。**
    pub fn write_file(&self, path: &std::path::Path, content: &str) -> FileOutcome {
        self.sandbox.write_file(self.mode, path, content)
    }

    /// 把工具给的路径解析成绝对路径（相对路径按工作区解析）。
    ///
    /// 单独抽出来是因为**路径解析必须与沙箱判定用同一套语义** ——
    /// 若工具自己拼路径、沙箱另算一遍，两者对"这是哪个文件"的答案可能不一致。
    pub fn resolve(&self, path: &str) -> std::path::PathBuf {
        let p = std::path::Path::new(path);
        if p.is_absolute() { p.to_path_buf() } else { self.cwd.join(p) }
    }
}

/// 按字节上限截断，且**不切开 UTF-8 码点**。
///
/// 直接 `&s[..n]` 在非字符边界会 panic —— 这是 Rust 里截断字符串的经典坑。
/// 必须回退到最近的字符边界。
pub fn truncate_utf8(s: &str, max_bytes: usize) -> (&str, bool) {
    if s.len() <= max_bytes {
        return (s, false);
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (&s[..end], true)
}

pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    /// 给模型看的说明（进入提示词，须字节稳定）。
    fn describe(&self) -> String;
    /// 本工具在**给定参数下**的语义类别。参数可影响分类
    /// （bash 的 `cat` 是读、`rm` 是写）。
    fn call_kind(&self, args: &Value) -> CallKind;
    fn execute(&self, args: &Value, ctx: &ToolCtx) -> ToolOutput;
}

/// 用 BTreeMap 而非 HashMap —— 保证工具描述顺序稳定（提示词缓存命中的前提）
pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self { Self { tools: BTreeMap::new() } }
    pub fn register(&mut self, t: Arc<dyn Tool>) { self.tools.insert(t.name().to_string(), t); }

    /// 固定顺序输出描述。**顺序必须字节稳定**，否则 prompt cache 全 miss。
    pub fn render_prompt(&self) -> String {
        self.tools.values().map(|t| t.describe()).collect::<Vec<_>>().join("\n")
    }

    /// 工具 schema 列表（顺序同上，稳定）。
    pub fn schemas(&self) -> Vec<ToolSchema> {
        self.tools
            .values()
            .map(|t| ToolSchema { name: t.name().into(), description: t.describe() })
            .collect()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> { self.tools.get(name).cloned() }
    pub fn len(&self) -> usize { self.tools.len() }
    pub fn is_empty(&self) -> bool { self.tools.is_empty() }
}

impl Default for ToolRegistry { fn default() -> Self { Self::new() } }

// ══════════════════════════════════════════════════════════════════════
// 扩展点 3/4：SandboxBackend ｜ 扩展点 4/4：SessionPersistence
// ══════════════════════════════════════════════════════════════════════

/// 沙箱：在某档安全语义下执行一条命令。
///
/// 参数用领域类型（`SandboxMode`），不用 OS 专有结构体 —— 实现按平台分，
/// 接口不变。
pub trait SandboxBackend: Send + Sync {
    /// 本后端对给定模式的**能力声明**，供降级判断（能力可缺，但不得静默失败）。
    fn supports(&self, mode: SandboxMode) -> bool;

    /// 执行一条命令。**越权必须被拒**（T4 断言的核心）。
    ///
    /// `limit_bytes` 是**单次输出上限，而且是契约的一部分**，不是事后补救：
    /// 实现必须在**读取过程中**就停止累积（边读边丢），否则内存早已被吃掉，
    /// 再截断毫无意义。这是"有界"从约定变成契约的关键。
    fn execute(&self, mode: SandboxMode, command: &str, limit_bytes: usize) -> SandboxOutcome;

    /// 在沙箱策略下写文件。**所有文件变更都必须经此。**
    ///
    /// # 为什么文件写入也属于沙箱，而不是"工具自己 std::fs"
    ///
    /// 若 `apply_patch` 直接调 `std::fs`，就会出现一个漏洞：
    /// **同一个"写"语义，经 shell 走 OS 沙箱、经工具却不受约束。**
    /// 那么 `workspace-write` 档就成了半张空头支票 —— 用 `apply_patch`
    /// 可以写到工作区外，而用 `bash` 不行。
    ///
    /// 把写入放进沙箱契据，使「沙箱是唯一的变更所有者」成为**结构事实**：
    /// 工具想改文件没有别的入口，实现也无从绕过策略。
    fn write_file(&self, mode: SandboxMode, path: &std::path::Path, content: &str) -> FileOutcome;
}

/// 一次文件写入的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOutcome {
    Written { bytes: usize },
    /// 策略拒绝（结构化事实，含原因）。
    Denied { reason: String },
    /// 策略允许但底层失败（IO 错误）。
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxOutcome {
    /// 执行完成。`truncated` 表示输出已按上限截断（**必须如实上报**，
    /// 否则上层会把"被截断的结果"当成完整结果）。
    Ran { stdout: String, truncated: bool },
    /// 因沙箱策略被拒（结构化事实，不是字符串报错）。
    Denied { reason: String },
}

/// 会话持久化：append-only 日志语义。
///
/// 命名纪律（SPI-First Step 1）：不使用 "JSONL"/"SQLite"/"S3" 等实现名，
/// 只用领域动词 append/load。实现可换而接口不变。
///
/// 注意：**格式版本化与实现替换是两件事**。日志格式（`v` 字段）必须向后兼容
/// 且可迁移；「用哪种介质存」才是本 SPI 的可替换点。
pub trait SessionPersistence: Send + Sync {
    /// 追加一条记录。**实现必须保证 append-only**：不得改写已写入的内容。
    fn append(&mut self, kind: &str, payload: Value) -> Result<Seq, PersistenceError>;
    /// 读回全部记录，顺序与写入一致。
    fn load(&self) -> Result<Vec<LoggedRecord>, PersistenceError>;
}

/// 日志中的一条记录（读回形态）。
#[derive(Debug, Clone, PartialEq)]
pub struct LoggedRecord {
    pub seq: Seq,
    pub kind: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistenceError {
    /// 日志被检测到非 append-only 改写（不可恢复，须报错而非静默）。
    Tampered,
    /// 底层不可用。
    Unavailable(String),
}

impl std::fmt::Display for PersistenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tampered => write!(f, "会话日志被检测到非 append-only 改写"),
            Self::Unavailable(why) => write!(f, "会话日志不可用：{why}"),
        }
    }
}
impl std::error::Error for PersistenceError {}

// ══════════════════════════════════════════════════════════════════════
// 闸门：沙箱（硬边界）× 审批（流程）× 文件编辑粒度
// ══════════════════════════════════════════════════════════════════════

/// 闸门判定结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    Allow,
    /// 必须暂停问人。`detail` 是给用户看的结构化说明。
    Ask { detail: String },
    /// 硬拒绝 —— **审批策略无法推翻**（沙箱是技术边界）。
    Deny { reason: String },
}

/// 三维闸门。
///
/// 判定顺序不可交换：
/// 1. **沙箱是硬边界** —— 越权直接 `Deny`，任何审批策略都推不翻。
/// 2. **文件编辑粒度** —— ZCode 的 Default 与 AutoEdit 在双轴上完全相同，
///    真实差异只在「文件编辑是否自动放行」。缺这一维，两档不可区分。
/// 3. **审批是流程** —— 何时暂停问人。
pub fn gate(kind: CallKind, res: ModeResolution) -> GateDecision {
    // ── 第 1 层：沙箱硬边界（不可被审批覆盖）──
    if res.sandbox == SandboxMode::ReadOnly && kind == CallKind::Write {
        return GateDecision::Deny {
            reason: "只读沙箱禁止任何写入（沙箱是技术边界，审批策略无法放行）".into(),
        };
    }

    // ── 第 2 层：文件编辑粒度（仅对写有效）──
    if kind == CallKind::Write && res.file_edit == FileEditPolicy::Auto {
        return GateDecision::Allow;
    }

    // ── 第 3 层：审批流程 ──
    match res.approval {
        ApprovalPolicy::Never => GateDecision::Allow,
        ApprovalPolicy::Untrusted => GateDecision::Ask {
            detail: format!("{}类调用在 untrusted 策略下需逐次确认", describe_kind(kind)),
        },
        ApprovalPolicy::OnRequest => match kind {
            // 只读与交互不打断用户
            CallKind::Read | CallKind::Interactive => GateDecision::Allow,
            CallKind::Write | CallKind::Network => GateDecision::Ask {
                detail: format!("{}类调用需确认", describe_kind(kind)),
            },
        },
        // OnFailure 的语义是"先执行，失败后再问"。执行前无法预判失败，故放行；
        // 失败重试时的追问属工具层工作，**当前未实现**（诚实边界）。
        ApprovalPolicy::OnFailure => GateDecision::Allow,
    }
}

fn describe_kind(kind: CallKind) -> &'static str {
    match kind {
        CallKind::Read => "读取",
        CallKind::Write => "写入",
        CallKind::Network => "网络",
        CallKind::Interactive => "交互",
    }
}

// ══════════════════════════════════════════════════════════════════════
// 内核：turn/step 主循环
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelError {
    /// 没有待审批的调用却收到 Approve。
    NoPendingApproval(ApprovalId),
    /// 持久化失败（含 append-only 被破坏）。
    Persistence(PersistenceError),
    /// 尚未实现的 Op（明确报错，不静默忽略）。
    Unimplemented(String),
    /// 上下文超上限：要求压缩，而不是静默丢消息。
    ///
    /// 为什么不静默丢弃最老的：模型的视角必须与日志一致（"模型可见即已落盘"）。
    /// 悄悄丢消息会让回放出的历史与实际请求不符 —— 那比报错危险得多。
    ContextBudgetExceeded { messages: usize, limit: usize },
}

impl std::fmt::Display for KernelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPendingApproval(id) => write!(f, "无待审批调用：{id}"),
            Self::Persistence(e) => write!(f, "{e}"),
            Self::Unimplemented(what) => write!(f, "该 Op 尚未实现：{what}"),
            Self::ContextBudgetExceeded { messages, limit } => write!(
                f,
                "上下文超上限（{messages} > {limit} 条），需压缩后再继续；压缩属 L4 职责，当前未实现"
            ),
        }
    }
}
impl std::error::Error for KernelError {}

/// 内核状态。**只有两态**：空闲、或卡在某个调用的审批上。
///
/// 刻意不设 `Planning`/`Executing` 等中间态 —— 那是「一轮正在进行」的内部细节，
/// 宿主不关心；对外暴露只会造成状态机与实现不同步。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelState {
    Idle,
    AwaitingApproval { id: ApprovalId },
}

/// 挂起点：同一步内，第 `index` 个调用等待审批。
///
/// 保存**整批调用**而非单个，是为了恢复后能继续执行同一步剩余调用 ——
/// 否则模型会看到"半个步骤"，工具结果顺序也会错乱。
#[derive(Debug, Clone)]
struct PendingApproval {
    calls: Vec<ToolInvocation>,
    index: usize,
}

pub struct Kernel {
    cfg: Config,
    tools: ToolRegistry,
    model: Box<dyn ModelProvider>,
    sandbox: Arc<dyn SandboxBackend>,
    persistence: Box<dyn SessionPersistence>,
    cwd: PathBuf,
    max_steps: usize,
    /// 单次工具调用的输出上限（内核级，工具不可放宽）
    max_output_bytes: usize,
    /// 上下文消息上限；超出即报错要求压缩，而非静默无限增长
    max_context_messages: usize,
    /// 系统提示词：构造时算一次。**必须字节稳定**（提示词缓存命中的前提），
    /// 且避免每步重新拼接。
    system_prompt: String,
    /// 工具 schema：构造时算一次，同样避免每步分配。
    tool_schemas: Vec<ToolSchema>,

    session_id: String,
    state: KernelState,
    /// 模型可见历史（派生物；真相在日志）
    messages: Vec<Message>,
    steps_this_turn: usize,
    turn_counter: u64,
    step_counter: u64,
    usage_in: u64,
    usage_out: u64,
    pending: Option<PendingApproval>,
    /// 本次 submit 产生的事件（宿主收取）
    outbox: Vec<EventMsg>,
}

impl Kernel {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: impl Into<String>,
        cfg: Config,
        tools: ToolRegistry,
        model: Box<dyn ModelProvider>,
        sandbox: Arc<dyn SandboxBackend>,
        persistence: Box<dyn SessionPersistence>,
        cwd: impl Into<PathBuf>,
    ) -> Self {
        let tools = tools;
        let tool_schemas = tools.schemas();
        let mut system_prompt =
            String::from("你是 NEO 的编码 agent。优先用工具核验事实，不要凭记忆断言。\n\n可用工具：\n");
        system_prompt.push_str(&tools.render_prompt());

        Self {
            cfg,
            tools,
            model,
            sandbox,
            persistence,
            cwd: cwd.into(),
            max_steps: DEFAULT_MAX_STEPS,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            max_context_messages: DEFAULT_MAX_CONTEXT_MESSAGES,
            system_prompt,
            tool_schemas,
            session_id: session_id.into(),
            state: KernelState::Idle,
            messages: Vec::new(),
            steps_this_turn: 0,
            turn_counter: 0,
            step_counter: 0,
            usage_in: 0,
            usage_out: 0,
            pending: None,
            outbox: Vec::new(),
        }
    }

    pub fn with_max_steps(mut self, n: usize) -> Self { self.max_steps = n; self }
    pub fn with_output_cap(mut self, bytes: usize) -> Self { self.max_output_bytes = bytes; self }
    pub fn with_context_cap(mut self, messages: usize) -> Self {
        self.max_context_messages = messages;
        self
    }
    /// 仅供测试：直接灌入历史消息，用于测量分配行为。
    ///
    /// 生产路径不得使用 —— 绕过了"模型可见即已落盘"的铁律。
    #[doc(hidden)]
    pub fn seed_history_for_test(&mut self, n: usize) {
        for i in 0..n {
            self.messages.push(Message::User(format!("seed-{i}")));
        }
    }

    /// 系统提示词（供测试断言字节稳定）。
    pub fn system_prompt(&self) -> &str { &self.system_prompt }
    pub fn state(&self) -> &KernelState { &self.state }
    pub fn messages(&self) -> &[Message] { &self.messages }
    pub fn session_id(&self) -> &str { &self.session_id }

    /// 当前生效的执行语义（三维）。
    ///
    /// **以 `exec_mode` 为唯一事实源**：五档模式是 UI 的唯一入口，避免了
    /// 「双轴与档位不一致」这一经典困惑（见 ADR-0004）。`Config` 上的
    /// `sandbox_mode`/`approval_policy` 字段不参与内核对轴的决定。
    pub fn resolution(&self) -> ModeResolution { resolve(self.cfg.exec_mode) }

    /// 读回已落盘的日志记录（供审计与回放测试）。
    ///
    /// 公开这个只读视图的理由：T2/T3 的断言对象就是**日志**，而不是内核内部状态。
    /// 如果测试只能看 `messages`，就无法区分"模型可见但没落盘"这类违规。
    pub fn log_records(&self) -> Vec<LoggedRecord> {
        self.persistence.load().unwrap_or_default()
    }

    /// 提交一个 Op，跑完它产生的事件并返回。
    ///
    /// 确定性契约：同一 Op 序列 + 同一 mock provider ⇒ 同一 EventMsg 序列（T2）。
    pub fn submit(&mut self, op: Op) -> Result<Vec<EventMsg>, KernelError> {
        self.outbox.clear();
        self.log("op", &op)?;

        match op {
            Op::UserTurn { text, refs } => {
                // 先检查预算再推入用户消息：否则会留下一条"无法被处理"的消息，
                // 让历史与日志都多出一条实际没发出去的输入。
                self.check_context_budget()?;
                self.turn_counter += 1;
                self.steps_this_turn = 0;
                self.usage_in = 0;
                self.usage_out = 0;
                let turn_id = format!("turn-{}", self.turn_counter);

                let started = EventMsg::TurnStarted { turn_id };
                self.emit_and_log(&started)?;

                // 引用（@ / # / / / $）作为用户输入的一部分进入历史。
                // 真实实现会在此把引用解析成具体内容；当前**只记录不解析**
                // （诚实边界：解析属 M1 后续工作）。
                let user_text =
                    if refs.is_empty() { text } else { format!("{text}\n[refs: {}]", refs.len()) };
                self.messages.push(Message::User(user_text));

                self.drive_steps()?;
            }

            Op::Approve { id, decision } => {
                let Some(pending) = self.pending.take() else {
                    return Err(KernelError::NoPendingApproval(id));
                };
                let call = pending.calls[pending.index].clone();
                self.state = KernelState::Idle;

                match decision {
                    Decision::Allow | Decision::AllowAlways => self.execute_one(&call)?,
                    Decision::Deny => {
                        let ev = EventMsg::ToolCallEnd { id: call.id.clone(), exit_code: -1 };
                        self.emit_and_log(&ev)?;
                        self.messages.push(Message::ToolResult {
                            id: call.id,
                            name: call.name,
                            output: denied_output("用户拒绝了该调用"),
                        });
                    }
                }
                // 继续同一步的剩余调用，然后进入下一步
                self.finish_step_from(&pending.calls, pending.index + 1)?;
            }

            Op::Interrupt => {
                // 中断只在**工具调用边界**生效（安全点），杜绝半写状态。
                self.pending = None;
                self.state = KernelState::Idle;
                self.emit_and_log(&EventMsg::Error { message: "已中断".into() })?;
            }

            Op::ConfigureSession { patch } => {
                self.cfg = neo_config::merge(self.cfg.clone(), patch.into());
                let ev = EventMsg::SessionConfigured { session_id: self.session_id.clone() };
                self.emit_and_log(&ev)?;
            }

            Op::Shutdown => {
                self.emit_and_log(&EventMsg::ShutdownComplete)?;
            }

            other => return Err(KernelError::Unimplemented(format!("{other:?}"))),
        }

        Ok(std::mem::take(&mut self.outbox))
    }

    // ── 主循环 ────────────────────────────────────────────────────────

    /// 跑完本轮：反复「模型一步 → 执行其工具调用」，直到模型不再要工具。
    fn drive_steps(&mut self) -> Result<(), KernelError> {
        loop {
            if self.steps_this_turn >= self.max_steps {
                let msg = EventMsg::Error { message: format!("超出步数预算（{} 步）", self.max_steps) };
                self.emit_and_log(&msg)?;
                break;
            }
            self.check_context_budget()?;
            self.steps_this_turn += 1;
            self.step_counter += 1;

            let (text, calls) = self.model_step()?;
            self.messages.push(Message::Assistant { text, tool_calls: calls.clone() });

            if calls.is_empty() {
                break; // 模型不再要工具 → 本轮结束
            }
            if calls.len() > MAX_TOOL_CALLS_PER_STEP {
                let msg = EventMsg::Error {
                    message: format!("单步工具调用过多（{} > {}）", calls.len(), MAX_TOOL_CALLS_PER_STEP),
                };
                self.emit_and_log(&msg)?;
                break;
            }
            match self.execute_from(&calls, 0)? {
                ExecOutcome::Done => continue,   // 工具欠一次请求 → 下一步
                ExecOutcome::Suspended => return Ok(()), // 挂审批，本轮暂停
            }
        }

        // 只在没有挂起时结束本轮
        if matches!(self.state, KernelState::Idle) {
            let done = EventMsg::TurnComplete { input_tokens: self.usage_in, output_tokens: self.usage_out };
            self.emit_and_log(&done)?;
        }
        Ok(())
    }

    /// 一次模型请求：组装 → 流式消费 → 返回（文本, 工具调用）。
    ///
    /// **零拷贝**：历史用 `mem::take` 临时移出，而不是 `clone` ——
    /// 每步克隆整份历史会让一轮退化到 O(N²)，长会话下是主要热点。
    /// 移出后 `self` 可自由可变借用（落盘/入队），跑完再放回。
    fn model_step(&mut self) -> Result<(String, Vec<ToolInvocation>), KernelError> {
        let messages = std::mem::take(&mut self.messages);

        let (text, calls, result) = {
            let request = ModelRequest {
                system: &self.system_prompt,
                messages: &messages,
                tools: &self.tool_schemas,
            };

            let mut text = String::new();
            let mut calls = Vec::new();
            let mut result: Result<(), KernelError> = Ok(());

            for delta in self.model.stream(&request) {
                match delta {
                    ModelDelta::Text(chunk) => {
                        text.push_str(&chunk);
                        let ev = EventMsg::AgentMessageDelta { delta: chunk };
                        if let Err(e) = self.emit_and_log(&ev) {
                            result = Err(e);
                            break;
                        }
                    }
                    ModelDelta::ToolCall(call) => {
                        let ev =
                            EventMsg::ToolCallBegin { id: call.id.clone(), name: call.name.clone() };
                        if let Err(e) = self.emit_and_log(&ev) {
                            result = Err(e);
                            break;
                        }
                        calls.push(call);
                    }
                    ModelDelta::Usage { input_tokens, output_tokens } => {
                        self.usage_in += input_tokens;
                        self.usage_out += output_tokens;
                    }
                }
            }
            (text, calls, result)
        };

        self.messages = messages; // 放回
        result?;
        self.emit_and_log(&EventMsg::AgentMessageDone { text: text.clone() })?;
        Ok((text, calls))
    }

    fn execute_from(&mut self, calls: &[ToolInvocation], start: usize) -> Result<ExecOutcome, KernelError> {
        for index in start..calls.len() {
            let call = calls[index].clone();
            let kind = self.classify(&call);
            match gate(kind, self.resolution()) {
                GateDecision::Allow => self.execute_one(&call)?,
                GateDecision::Deny { reason } => {
                    let ev = EventMsg::ToolCallEnd { id: call.id.clone(), exit_code: -1 };
                    self.emit_and_log(&ev)?;
                    self.messages.push(Message::ToolResult {
                        id: call.id,
                        name: call.name,
                        output: denied_output(&reason),
                    });
                }
                GateDecision::Ask { detail } => {
                    // 确定性 id：由 (turn, step, index) 派生，不用随机/时钟
                    let id = format!("approval-{}-{}-{}", self.turn_counter, self.step_counter, index);
                    let ev = EventMsg::ApprovalRequest { id: id.clone(), detail };
                    self.emit_and_log(&ev)?;
                    self.pending = Some(PendingApproval { calls: calls.to_vec(), index });
                    self.state = KernelState::AwaitingApproval { id };
                    return Ok(ExecOutcome::Suspended);
                }
            }
        }
        Ok(ExecOutcome::Done)
    }

    /// 恢复后继续同一步剩余调用；跑完则进入下一步。
    fn finish_step_from(&mut self, calls: &[ToolInvocation], start: usize) -> Result<(), KernelError> {
        if start < calls.len() {
            match self.execute_from(calls, start)? {
                ExecOutcome::Done => {}
                ExecOutcome::Suspended => return Ok(()),
            }
        }
        self.drive_steps()
    }

    /// 上下文预算检查。超限即报错，绝不静默丢弃。
    fn check_context_budget(&self) -> Result<(), KernelError> {
        let n = self.messages.len();
        if n >= self.max_context_messages {
            return Err(KernelError::ContextBudgetExceeded {
                messages: n,
                limit: self.max_context_messages,
            });
        }
        Ok(())
    }

    /// 工具分类：优先问工具自己；工具不认识该名字则保守判为 Write
    /// （宁可多问一次，不可漏放一次写操作）。
    fn classify(&self, call: &ToolInvocation) -> CallKind {
        match self.tools.get(&call.name) {
            Some(t) => t.call_kind(&call.arguments),
            None => CallKind::Write,
        }
    }

    /// 真正执行一个调用：经沙箱、落日志、进历史。
    fn execute_one(&mut self, call: &ToolInvocation) -> Result<(), KernelError> {
        let output = match self.tools.get(&call.name) {
            Some(tool) => {
                let ctx = ToolCtx {
                    sandbox: self.sandbox.as_ref(),
                    mode: self.resolution().sandbox,
                    cwd: &self.cwd,
                    max_output_bytes: self.max_output_bytes,
                };
                tool.execute(&call.arguments, &ctx)
            }
            None => ToolOutput {
                exit_code: -1,
                stdout: String::new(),
                stderr: format!("未知工具：{}", call.name),
                truncated: false,
            },
        };
        let ev = EventMsg::ToolCallEnd { id: call.id.clone(), exit_code: output.exit_code };
        self.emit_and_log(&ev)?;
        self.messages.push(Message::ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            output,
        });
        Ok(())
    }

    fn emit_and_log(&mut self, ev: &EventMsg) -> Result<(), KernelError> {
        self.outbox.push(ev.clone());
        self.log("event", ev)
    }

    /// 落盘。**任何模型可见内容都必须经过这里**，否则回放不成立。
    fn log(&mut self, kind: &str, payload: &impl serde::Serialize) -> Result<(), KernelError> {
        let value = serde_json::to_value(payload)
            .map_err(|e| KernelError::Persistence(PersistenceError::Unavailable(e.to_string())))?;
        self.persistence.append(kind, value).map_err(KernelError::Persistence)?;
        Ok(())
    }
}

enum ExecOutcome { Done, Suspended }

fn denied_output(reason: &str) -> ToolOutput {
    ToolOutput { exit_code: -1, stdout: String::new(), stderr: reason.into(), truncated: false }
}

// ══════════════════════════════════════════════════════════════════════
// 宿主 SPI：契据在 L2，实现在 L5
// ══════════════════════════════════════════════════════════════════════

/// 宿主能力自描述。存在的意义：**让降级变成数据驱动**。
/// 没有它，工具就得写 `if is_tui {..} else {..}` —— 那正是 Proteus 反对的
/// "业务代码里出现平台分支"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostCapabilities {
    pub images: ImageSupport,
    pub rich_text: bool,
    pub interactive_prompt: bool,
    pub diffs: DiffSupport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageSupport { None, Inline, External }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffSupport { None, Text, Hunk }

/// 宿主后端契据。实现在 L5；宿主之间不得互相依赖（check_architecture 强制）。
///
/// 收 `&EventMsg` 而非 JSON 字符串：宿主与内核同进程，
/// 中间加一层序列化只是白白的开销与出错点（解析失败、字段漂移）。
pub trait HostBackend: Send {
    fn id(&self) -> &'static str;
    fn capabilities(&self) -> HostCapabilities;
    /// 消费一条事件。返回 Err 表示本宿主无法处理该事件 —— T6 断言 (a) 的判据。
    fn consume(&mut self, event: &EventMsg) -> Result<(), String>;
    /// 本宿主已向用户传达的事实集合。**T6 断言 (b) 的比较对象。**
    ///
    /// 返回协议层的 [`Fact`] 而非渲染后的字符串：
    /// "等价"要能机器判定，前提是两边用同一套语义词表。
    fn facts(&self) -> Vec<Fact>;
}

/// 宿主注册表：内核只认契据，不认具体后端。
pub struct HostRegistry { hosts: Vec<Box<dyn HostBackend>> }

impl HostRegistry {
    pub fn new() -> Self { Self { hosts: Vec::new() } }
    pub fn register(&mut self, h: Box<dyn HostBackend>) { self.hosts.push(h); }
    pub fn ids(&self) -> Vec<&'static str> { self.hosts.iter().map(|h| h.id()).collect() }

    /// 把同一事件流喂给所有宿主。T6 的运行时形态。
    pub fn broadcast(&mut self, event: &EventMsg) -> Vec<(&'static str, Result<(), String>)> {
        self.hosts.iter_mut().map(|h| (h.id(), h.consume(event))).collect()
    }
}

impl Default for HostRegistry { fn default() -> Self { Self::new() } }
