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
pub mod models;
pub mod skills;
pub mod instructions;

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
// 扩展点：Compactor（上下文压缩策略）
// ══════════════════════════════════════════════════════════════════════
//
// # 为什么契据在 L2、实现在 L4
//
// 架构规矩是「依赖只能向下」，所以 L2 不能 import L4。但"压哪几条消息"
// 确实是 L4 的策略问题（编排层决定保留多少上下文才够继续）。
// 解法与 ModelProvider/Tool 一致：**内核定义 seam，上层实现并注入**。
// 这样策略可替换（换压缩算法不动内核），方向也不破。

/// 上下文压缩策略。
pub trait Compactor: Send + Sync {
    /// 给定当前消息序列，返回 `(摘要文本, 保留起点下标)`。
    ///
    /// `None` 表示**压不了/不该压**（例如切不出安全边界）。内核据此
    /// 如实报错，而不是硬压 —— 产出非法请求比不压更糟。
    ///
    /// 关键约束（实现方必须遵守）：`keep_from` 必须落在**轮的起点**
    /// （用户消息）上。否则会留下没有对应 tool_call 的 tool 消息，
    /// 真实 provider 会以 400 拒绝。
    fn plan(&self, messages: &[Message]) -> Option<(String, usize)>;
}

/// 无压缩器：`Op::Compact` 会如实报告"未配置压缩策略"。
pub struct NoCompactor;

impl Compactor for NoCompactor {
    fn plan(&self, _messages: &[Message]) -> Option<(String, usize)> {
        None
    }
}

// ══════════════════════════════════════════════════════════════════════
// 扩展点 2/4：Tool
// ══════════════════════════════════════════════════════════════════════

/// 工具调用的语义类别 —— 闸门据此判定，而不是按工具名字符串猜。
///
/// 旧版用 `if call == "bash"` 判断，脆弱：新增工具就漏判、改名就失效。
/// 类别是**工具自己的知识**，应由工具声明。
// `PartialOrd, Ord` 是为了放进 `BTreeSet`（会话级"总是允许"的类别集合）——
// 顺序本身无意义，只需要一个确定的序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

/// 引用解析结果：人类可读摘要 + 要注入模型请求的上下文块。
///
/// 分开是因为两者受众不同：`summary` 给用户与转录（"哪条读到了"），
/// `block` 给模型（文件/技能正文）。合并成一个字符串会导致
/// 要么用户看到几百行文件内容，要么模型看到"📄 已注入"却没拿到正文。
#[derive(Debug, Clone, Default)]
pub struct RefResolution {
    pub summary: Vec<String>,
    pub block: Option<String>,
}

/// 单引号 shell 转义（POSIX 语义）：把 `'` 换成 `'\''`。
///
/// 独立成函数是为了**只在一处**做转义 —— 转义逻辑散落各处时，
/// 漏掉一处的后果是把用户输入当命令执行。
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// 按行范围截取（1 基，闭区间），并报告是否因**上限**截断。
///
/// `lines = None` 表示整个文件。与 `truncate_utf8` 的分工：
/// 这里管"用户要的是哪几行"，那里管"字节上限"。
/// 返回 `(文本, 是否截断)`；越界的范围**不报错**，取交集即可
/// （文件比引用时短是常态：用户记行号总会偏）。
pub fn truncate_lines(s: &str, lines: Option<(usize, usize)>, max_bytes: usize) -> (String, bool) {
    let selected: String = match lines {
        None => s.to_string(),
        Some((a, b)) => s
            .lines()
            .skip(a.saturating_sub(1))
            .take(b.saturating_sub(a.saturating_sub(1)))
            .collect::<Vec<_>>()
            .join("\n"),
    };
    let (text, cut) = truncate_utf8(&selected, max_bytes);
    (text.to_string(), cut)
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
    /// 执行前可展示的改动预览（路径, diff）。
    ///
    /// 为什么放在工具而不是内核：**只有工具知道自己的参数怎么变成改动**。
    /// 内核只负责"要审批时先把预览发给宿主"，不解析任何工具特有的参数格式。
    /// 默认 `None` —— 不改文件的工具（bash/grep）无需实现。
    fn preview(&self, _args: &Value) -> Option<(String, String)> {
        None
    }
    /// 执行后要公告的事件（如 `todowrite` 更新任务清单）。
    ///
    /// 与 `preview` 对称：preview 是"执行前给用户看什么"，
    /// report 是"执行后要向宿主声明什么"。都由**工具**决定 ——
    /// 只有它知道自己的参数意味着什么状态变化；内核只负责转发。
    /// 默认空：绝大多数工具不产生协议事件。
    fn report(&self, _args: &Value) -> Vec<EventMsg> {
        Vec::new()
    }
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
    /// 压缩无法执行（未配策略 / 切不出安全边界）。
    CompactUnavailable(String),
    /// 模型切换失败（名字不存在等）。
    ///
    /// 单独变体而不是复用 `Unimplemented`：一个说"能力还没有"，
    /// 一个说"你要的模型不存在"，混用会让用户以为功能缺失。
    ModelSwitch(String),
    /// 请求回退的轮次超过已有历史（参数不合法，不是"没实现"）。
    ///
    /// 与 `Unimplemented` 分开：一个说"这个能力还没有"，一个说"你的参数不成立"。
    /// 混用会让用户以为功能缺失，而实际只是 requested too far back。
    RewindTooFar { requested: usize, available: usize },
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
            Self::CompactUnavailable(msg) => write!(f, "{msg}"),
            Self::ModelSwitch(msg) => write!(f, "{msg}"),
            Self::RewindTooFar { requested, available } => write!(
                f,
                "无法回退 {requested} 轮：当前只有 {available} 个用户轮次"
            ),
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
    /// 模型注册表（多 provider + 当前选中）。运行时可切换。
    models: crate::models::ModelRegistry,
    /// 上下文压缩策略（L4 实现，构造时注入；缺省不压缩）
    compactor: Box<dyn Compactor>,
    sandbox: Arc<dyn SandboxBackend>,
    persistence: Box<dyn SessionPersistence>,
    cwd: PathBuf,
    max_steps: usize,
    /// 单次工具调用的输出上限（内核级，工具不可放宽）
    max_output_bytes: usize,
    /// 上下文消息上限；超出即报错要求压缩，而非静默无限增长
    max_context_messages: usize,
    /// 累计的文件改动（path -> (additions, deletions)）。BTreeMap 保证遍历顺序稳定。
    file_changes: std::collections::BTreeMap<String, (usize, usize)>,
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
    /// 本会话内**永久放行**的调用类别（`Decision::AllowAlways` 的结果）。
    ///
    /// # 为什么需要它
    ///
    /// 之前 `AllowAlways` 与 `Allow` 走同一分支 —— 协议声明了这个语义，
    /// 内核却忽略它，于是**每个工具调用都重新问一遍**。用户连续回答十几次
    /// `y` 仍被反复询问（真实反馈），因为每次都是新的门禁判定，没有会话级记忆。
    ///
    /// 只记**类别**（Read/Write/Network/Interactive），不记具体命令：
    /// 记具体命令等于把"放行这一次"伪装成"放行这类"，而用户点的是后者。
    granted: std::collections::BTreeSet<CallKind>,
    /// 技能注册表（`$skill` 引用的解析来源）。构造时注入，缺省为空。
    skills: crate::skills::SkillRegistry,
    /// 项目指令（AGENTS.md 级联）。构造时注入，缺省为空。
    instructions: crate::instructions::Instructions,
    /// 指令是否已落日志（**只落一次**：它是系统提示词的组成，每轮重复落盘纯属放大日志）。
    instructions_logged: bool,
    /// 本次 submit 产生的事件（宿主收取）
    outbox: Vec<EventMsg>,
}

impl Kernel {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: impl Into<String>,
        cfg: Config,
        tools: ToolRegistry,
        model: crate::models::ModelRegistry,
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
            models: model,
            compactor: Box::new(NoCompactor),
            sandbox,
            persistence,
            cwd: cwd.into(),
            max_steps: DEFAULT_MAX_STEPS,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            max_context_messages: DEFAULT_MAX_CONTEXT_MESSAGES,
            file_changes: std::collections::BTreeMap::new(),
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
            granted: std::collections::BTreeSet::new(),
            skills: crate::skills::SkillRegistry::new(),
            instructions: crate::instructions::Instructions::default(),
            instructions_logged: false,
            outbox: Vec::new(),
        }
    }

    pub fn with_max_steps(mut self, n: usize) -> Self { self.max_steps = n; self }
    pub fn with_output_cap(mut self, bytes: usize) -> Self { self.max_output_bytes = bytes; self }
    /// 注入技能注册表（`$skill` 引用据此解析）。缺省为空 = `$x` 如实报告找不到。
    pub fn with_skills(mut self, skills: crate::skills::SkillRegistry) -> Self {
        self.skills = skills;
        self
    }

    /// 注入项目指令（AGENTS.md 级联）。**重建系统提示词** ——
    /// 指令是提示词的组成，不重建就不会真正进入请求。
    pub fn with_instructions(mut self, ins: crate::instructions::Instructions) -> Self {
        self.instructions = ins;
        self.refresh_system_prompt();
        self
    }

    /// 按当前指令重新拼系统提示词。**必须字节稳定**（提示词缓存命中的前提）：
    /// 拼接顺序固定，不做任何基于内容的排序或格式化。
    fn refresh_system_prompt(&mut self) {
        let mut p = String::from("你是 NEO 的编码 agent。优先用工具核验事实，不要凭记忆断言。\n\n可用工具：\n");
        p.push_str(&self.tools.render_prompt());
        if !self.instructions.is_empty() {
            p.push_str("\n\n");
            p.push_str(&self.instructions.block);
        }
        self.system_prompt = p;
    }
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
        // 指令组成系统提示词（模型可见），必须在**第一次**请求前落盘。
        // 只落一次：它每轮都进请求，逐轮重复落盘会把日志放大到无意义。
        self.ensure_instructions_logged()?;

        match op {
            Op::UserTurn { text, refs } => {
                self.begin_turn(text, refs)?;
                self.drive_steps()?;
            }
            // 逐帧宿主：只做开场（回显/引用/入历史），不驱动。
            Op::BeginTurn { text, refs } => {
                self.begin_turn(text, refs)?;
            }
            // 推进一步。到边界（TurnComplete / ApprovalRequest）宿主自行判断。
            Op::Pump => match self.step_once()? {
                StepOutcome::More | StepOutcome::Suspended => {}
                StepOutcome::Done => self.finish_turn_if_idle()?,
            },

            Op::Shell { command } => {
                // 与模型请求的工具调用走**同一条**执行路径（execute_one），
                // 因此沙箱、输出上限、截断标记、落盘全部自动一致 ——
                // 不为"用户直输的命令"另开一条旁路。
                let call = ToolInvocation {
                    id: format!("shell-{}", self.turn_counter),
                    name: "bash".to_string(),
                    // 参数名必须与 BashTool 读取的键一致（`cmd`）。
                    // 曾经这里写成 "command"，工具拿不到参数直接失败 ——
                    // 编译期无法发现，只有端到端跑 `!ls` 才暴露。
                    arguments: serde_json::json!({ "cmd": command }),
                };
                let begin = EventMsg::ToolCallBegin {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                };
                self.emit_and_log(&begin)?;
                self.execute_one(&call)?;
                // 注意：不发 turn_complete、也不驱动模型。
                // opencode 的 `!` 只把输出附到会话里，模型下一轮才看到它。
            }

            Op::Approve { id, decision } => {
                let Some(pending) = self.pending.take() else {
                    return Err(KernelError::NoPendingApproval(id));
                };
                let call = pending.calls[pending.index].clone();
                self.state = KernelState::Idle;

                match decision {
                    Decision::Allow => self.execute_one(&call)?,
                    Decision::AllowAlways => {
                        // 记住**类别**：之后同类调用不再问。
                        // 若只记这一条命令，用户下次仍会被问 —— 那就等于没实现。
                        let kind = self.classify(&call);
                        self.granted.insert(kind);
                        self.execute_one(&call)?;
                    }
                    Decision::Deny => {
                        let ev = EventMsg::ToolCallEnd { id: call.id.clone(), exit_code: -1, stdout: String::new(), stderr: String::new(), truncated: false };
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

            Op::Rewind { turns } => {
                // 找到倒数第 `turns` 个用户消息的下标，从那里截断。
                // 用"用户消息"当轮次边界，而不是"助手消息"—— 一轮可能包含
                // 多个助手消息（工具调用），只有用户消息才是轮的起点。
                let mut seen = 0usize;
                let mut cut = None;
                for (i, m) in self.messages.iter().enumerate().rev() {
                    if matches!(m, Message::User(_)) {
                        seen += 1;
                        if seen == turns {
                            cut = Some(i);
                            break;
                        }
                    }
                }
                let Some(cut) = cut else {
                    return Err(KernelError::RewindTooFar { requested: turns, available: seen });
                };
                let removed = self.messages.len() - cut;
                self.messages.truncate(cut);
                // 状态回到空闲：回退时可能在等审批，那批调用已经没有意义
                self.pending = None;
                self.state = KernelState::Idle;
                let ev = EventMsg::Rewound {
                    turns,
                    removed_messages: removed,
                    files_kept: self.file_changes.len(),
                };
                self.emit_and_log(&ev)?;
            }

            Op::Interrupt => {
                // 中断只在**工具调用边界**生效（安全点），杜绝半写状态。
                self.pending = None;
                self.state = KernelState::Idle;
                self.emit_and_log(&EventMsg::Error { message: "已中断".into() })?;
            }

            Op::ConfigureSession { patch } => {
                // 模型切换要**先校验再改配置**：若名字不存在，报错并保持原样，
                // 而不是把 cfg.model 改成不存在的名字（那会让会话日志说谎）。
                if let Some(want) = patch.model.as_deref() {
                    self.models
                        .switch(want)
                        .map_err(KernelError::ModelSwitch)?;
                    let ev = EventMsg::ModelSwitched {
                        model: want.to_string(),
                        context_limit: self.models.current_context_limit(),
                    };
                    self.emit_and_log(&ev)?;
                }
                self.cfg = neo_config::merge(self.cfg.clone(), patch.into());
                // cfg.model 与实际生效的 provider 保持一致（避免两处各说一套）
                self.cfg.model = self.models.current().to_string();
                let ev = EventMsg::SessionConfigured { session_id: self.session_id.clone() };
                self.emit_and_log(&ev)?;
            }

            Op::Compact => {
                // 压缩 = 用一条摘要替换掉前缀消息。**不静默丢弃**：
                // 摘要本身也落日志（模型可见即已落盘），且发事件让宿主知道
                // 上下文变了 —— 否则用户会奇怪"模型怎么忘了前面"。
                let before = self.messages.len();
                match self.compactor.plan(&self.messages) {
                    Some((summary, keep_from)) => {
                        if keep_from == 0 || keep_from > before {
                            // 策略给的边界不合法：拒绝执行而不是冒险砍坏历史
                            return Err(KernelError::CompactUnavailable(format!(
                                "压缩策略给出的保留点 {keep_from} 不合法（当前 {before} 条）"
                            )));
                        }
                        // 被摘要掉的是**前缀** `[0, keep_from)`，共 keep_from 条。
                        // 曾写成 `before - keep_from` —— 那算出来的是**保留下来的**
                        // 条数（保留 4 条却报"压缩 4 条"，而实际压了 12 条），
                        // 事件里的数字会误导用户，回放也会因此重演错。
                        let removed = keep_from;
                        let tail: Vec<Message> = self.messages[keep_from..].to_vec();
                        self.messages.clear();
                        self.messages.push(Message::System(summary.clone()));
                        self.messages.extend(tail);
                        let ev = EventMsg::ContextCompacted {
                            removed_messages: removed,
                            summary,
                        };
                        self.emit_and_log(&ev)?;
                    }
                    None => {
                        // 说清楚是"没配策略"还是"暂时压不了"，用户才知道怎么办
                        return Err(KernelError::CompactUnavailable(format!(
                            "当前无法压缩（{before} 条消息）。可能原因：未配置压缩策略，                             或切不出安全的轮次边界（需至少一轮完整对话可压）"
                        )));
                    }
                }
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
    /// 开场一轮：检查预算 → 回显 → 解析引用 → 推入历史。**不驱动**。
    ///
    /// 与 `drive_steps` 分开是为了让逐帧宿主能"先看到自己发了什么"，
    /// 再一步步推进 —— 整轮可能要多次网络往返，期间界面必须还能重绘。
    fn begin_turn(&mut self, text: String, refs: Vec<ContextRef>) -> Result<(), KernelError> {
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

                // 引用（@ / # / / / $）解析成具体内容，随用户消息一起进历史。
                //
                // 解析失败**不阻断本轮** —— 一条引用的路径打错不该让整个提问发不出去，
                // 但要如实把失败写进事件与摘要，让模型和用户都知道"这条没读到"。
                let resolution = self.resolve_refs(&refs);
                let user_text = match &resolution.block {
                    Some(block) => format!("{text}\n\n{block}"),
                    None => text.clone(),
                };
                // 先回显**用户原话**（不含注入块 —— 用户要看到自己打的字，
                // 而不是几百行文件正文），再报 turn 开始：转录顺序与用户感知一致。
                let echo = EventMsg::UserSubmitted { text: text.clone() };
                self.emit_and_log(&echo)?;
                // 注入块单独成事件：它模型可见（必须落盘），但用户不必在转录里读到。
                // 紧接着 UserSubmitted 发出，回放时"把块拼到刚推入的用户消息后"。
                //
                // **有摘要就要发**（哪怕 block 为空）：`$nope` 找不到技能时没有块，
                // 但"未找到，可用的是 X/Y"这条恰恰是用户唯一能得到的反馈 ——
                // 只在有块时发事件会让失败静默（本功能的第一个 bug）。
                if !resolution.summary.is_empty() {
                    let ev = EventMsg::RefsResolved {
                        summary: resolution.summary.clone(),
                        block: resolution.block.clone().unwrap_or_default(),
                    };
                    self.emit_and_log(&ev)?;
                }
                self.messages.push(Message::User(user_text));
        Ok(())
    }

    /// 跑完整轮（`submit(Op::UserTurn)` 用）。逐帧宿主用 `begin_turn` + `pump_step`。
    fn drive_steps(&mut self) -> Result<(), KernelError> {
        loop {
            match self.step_once()? {
                StepOutcome::More => continue,
                StepOutcome::Suspended => return Ok(()), // 挂审批，本轮暂停（不结束）
                StepOutcome::Done => break,
            }
        }
        self.finish_turn_if_idle()
    }

    /// 推进**一步**：一次模型请求 + 它要求的工具执行。
    ///
    /// 拆出来是为了让宿主能逐帧推进（每步之后重绘），而不是等整轮结束 ——
    /// 整轮可能包含多次网络往返，期间界面完全冻结（真实反馈："像卡死"）。
    fn step_once(&mut self) -> Result<StepOutcome, KernelError> {
        if self.steps_this_turn >= self.max_steps {
            let msg = EventMsg::Error { message: format!("超出步数预算（{} 步）", self.max_steps) };
            self.emit_and_log(&msg)?;
            return Ok(StepOutcome::Done);
        }
        self.check_context_budget()?;
        self.steps_this_turn += 1;
        self.step_counter += 1;

        let (text, calls) = self.model_step()?;
        self.messages.push(Message::Assistant { text, tool_calls: calls.clone() });

        if calls.is_empty() {
            return Ok(StepOutcome::Done); // 模型不再要工具 → 本轮结束
        }
        if calls.len() > MAX_TOOL_CALLS_PER_STEP {
            let msg = EventMsg::Error {
                message: format!("单步工具调用过多（{} > {}）", calls.len(), MAX_TOOL_CALLS_PER_STEP),
            };
            self.emit_and_log(&msg)?;
            return Ok(StepOutcome::Done);
        }
        match self.execute_from(&calls, 0)? {
            ExecOutcome::Done => Ok(StepOutcome::More), // 工具欠一次请求 → 下一步
            ExecOutcome::Suspended => Ok(StepOutcome::Suspended),
        }
    }

    /// 没有挂起时结束本轮（发 TurnComplete）。
    fn finish_turn_if_idle(&mut self) -> Result<(), KernelError> {
        if matches!(self.state, KernelState::Idle) {
            let done = EventMsg::TurnComplete {
                input_tokens: self.usage_in,
                output_tokens: self.usage_out,
            };
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

            for delta in self.models.current_provider().stream(&request) {
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
                        let ev = EventMsg::ToolCallBegin {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            arguments: call.arguments.clone(),
                        };
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
            // 会话级放行优先于门禁：用户对这类调用说过"总是允许"。
            // **沙箱硬边界仍是硬边界** —— 只读档下写入会被 gate 拒，
            // 但这里要先看门禁，不能因为说过"总是允许"就越过沙箱拒绝。
            let decision = gate(kind, self.resolution());
            let decision = match decision {
                GateDecision::Ask { .. } if self.granted.contains(&kind) => GateDecision::Allow,
                other => other,
            };
            match decision {
                GateDecision::Allow => self.execute_one(&call)?,
                GateDecision::Deny { reason } => {
                    let ev = EventMsg::ToolCallEnd { id: call.id.clone(), exit_code: -1, stdout: String::new(), stderr: String::new(), truncated: false };
                    self.emit_and_log(&ev)?;
                    self.messages.push(Message::ToolResult {
                        id: call.id,
                        name: call.name,
                        output: denied_output(&reason),
                    });
                }
                GateDecision::Ask { detail } => {
                    // 审批前先给**改动的具体内容**：只说"写入类调用需确认"，
                    // 用户是在盲批 —— 不知道改哪个文件、改了什么。
                    // 预览由工具提供（只有它知道参数怎么变成改动），内核只转发。
                    if let Some(tool) = self.tools.get(&call.name) {
                        if let Some((path, diff)) = tool.preview(&call.arguments) {
                            let ev = EventMsg::PatchProposed { path, diff };
                            self.emit_and_log(&ev)?;
                        }
                    }
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

    /// 切换到一个**已存在**的会话：换 id、换持久化目标、重建历史。
    ///
    /// 为什么不重建 `Kernel`：模型注册表、工具集、沙箱都是同一个，
    /// 换会话只影响"记到哪、历史是什么"。重建整个内核会丢掉这些装配，
    /// 且让调用方重复一遍构造代码（容易两处不一致）。
    ///
    /// **历史必须重建**：不重建就只是换了个文件名，模型看不到之前的对话，
    /// 转录也是空的 —— 那与"切换会话"的语义不符。
    pub fn switch_session(
        &mut self,
        session_id: impl Into<String>,
        persistence: Box<dyn SessionPersistence>,
    ) -> usize {
        self.session_id = session_id.into();
        self.persistence = persistence;
        self.state = KernelState::Idle;
        self.pending = None;
        self.outbox.clear();
        // 从现在起的轮次号独立（每个会话自己的轮次序列）
        self.turn_counter = 0;
        self.step_counter = 0;
        self.steps_this_turn = 0;
        let logs = self.persistence.load().unwrap_or_default();
        self.rebuild_from_log(&logs)
    }

    /// 注入压缩策略（链式，构造后立即调用；不注入则 `/compact` 如实报未配置）。
    pub fn with_compactor(mut self, c: Box<dyn Compactor>) -> Self {
        self.compactor = c;
        self
    }

    /// 取当前会话已落盘的日志（供重建与会话切换使用）。
    pub fn log_for_test(&self) -> Vec<LoggedRecord> {
        self.persistence.load().unwrap_or_default()
    }

    /// 从会话日志**重建对话历史**（会话切换/进程重启后继续的前提）。
    ///
    /// # 为什么必须能重建
    ///
    /// 这是 AGENTS.md 那条约束的落实：「凡进入模型请求的内容都要能从会话日志
    /// 重建」。若不能重建，切换会话只能得到一段空历史 —— 模型不知道之前聊过
    /// 什么，用户看到的转录也与实际不符。
    ///
    /// # 重建规则（与 `messages` 的构造一一对应）
    ///
    /// - `op: UserTurn` → `Message::User`（用户消息是轮的起点）
    /// - `event: AgentMessageDone` → `Message::Assistant`（**含该步的工具调用**）
    /// - `event: ToolCallEnd` → `Message::ToolResult`
    ///
    /// 工具调用靠 `ToolCallBegin`（含 arguments）配对 —— 这就是为什么
    /// `ToolCallBegin` 必须带参数：少了它，assistant 消息里的 tool_calls
    /// 无法补全，重建出的历史对**真实 provider** 是非法的（OpenAI 规范要求
    /// assistant 的 tool_call 与其后的 tool 结果成对出现）。
    ///
    /// 返回重建出的消息条数；`logs` 为空时不清空现有历史（无日志 ≠ 空会话）。
    pub fn rebuild_from_log(&mut self, logs: &[LoggedRecord]) -> usize {
        if logs.is_empty() {
            return 0;
        }
        let mut rebuilt: Vec<Message> = Vec::new();
        // 本步累积的工具调用：AgentMessageDone 到来时挂到 assistant 消息上
        let mut pending_calls: Vec<ToolInvocation> = Vec::new();

        for rec in logs {
            match rec.kind.as_str() {
                "op" => {
                    // 用户消息由紧随其后的 UserSubmitted 事件恢复（见下）。
                    // 这里**不推入** —— 否则同一条用户消息会被计两次。
                    // 引用也**不在此重解析**：文件内容早已变化，重读会得到
                    // 与当时不同的字节；真实注入的块在 RefsResolved 里（已落盘）。
                }
                "event" => {
                    match serde_json::from_value::<EventMsg>(rec.payload.clone()) {
                        // 指令是系统提示词的组成，回放必须还原**当时**那一份，
                        // 而不是重新读盘（AGENTS.md 可能已被改）。还原后重拼提示词，
                        // 否则回放出的请求与真实请求系统提示词不一致。
                        Ok(EventMsg::InstructionsLoaded { sources, block, truncated }) => {
                            self.instructions = crate::instructions::Instructions {
                                sources,
                                block,
                                truncated,
                            };
                            self.instructions_logged = true;
                            self.refresh_system_prompt();
                        }
                        Ok(EventMsg::UserSubmitted { text }) => {
                            rebuilt.push(Message::User(text));
                        }
                        // 注入块拼到刚推入的用户消息之后：submit 里就是
                        // `{text}\n\n{block}`，这里必须还原同一顺序。
                        Ok(EventMsg::RefsResolved { block, .. }) => {
                            if !block.is_empty() {
                                if let Some(Message::User(t)) = rebuilt.last_mut() {
                                    t.push_str("\n\n");
                                    t.push_str(&block);
                                }
                            }
                        }
                        Ok(EventMsg::ToolCallBegin { id, name, arguments }) => {
                            pending_calls.push(ToolInvocation { id, name, arguments });
                        }
                        Ok(EventMsg::AgentMessageDone { text }) => {
                            rebuilt.push(Message::Assistant {
                                text,
                                tool_calls: std::mem::take(&mut pending_calls),
                            });
                        }
                        Ok(EventMsg::ContextCompacted { removed_messages, summary }) => {
                            // 回放必须**重演**压缩动作：日志是 append-only 的，
                            // 无法改写旧记录，只能在重放时把"当时被摘要掉的前缀"
                            // 同样删掉并换上摘要。
                            //
                            // **不是 clear()**：那样会把压缩时**保留的**尾部也丢掉，
                            // 重建出的历史比真实历史短 —— 回放与真实请求不符，
                            // 正是"模型可见即已落盘"要防的事。
                            let n = removed_messages.min(rebuilt.len());
                            rebuilt.drain(0..n);
                            rebuilt.insert(0, Message::System(summary));
                        }
                        Ok(EventMsg::ToolCallEnd {
                            id,
                            exit_code,
                            stdout,
                            stderr,
                            truncated,
                        }) => {
                            let name = rebuilt
                                .iter()
                                .rev()
                                .find_map(|m| match m {
                                    Message::Assistant { tool_calls, .. } => tool_calls
                                        .iter()
                                        .find(|c| c.id == id)
                                        .map(|c| c.name.clone()),
                                    _ => None,
                                })
                                .unwrap_or_default();
                            rebuilt.push(Message::ToolResult {
                                id,
                                name,
                                output: ToolOutput { exit_code, stdout, stderr, truncated },
                            });
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        let n = rebuilt.len();
        self.messages = rebuilt;
        n
    }

    /// 当前模型名（宿主展示用）。
    pub fn current_model(&self) -> &str {
        self.models.current()
    }

    /// 运行时新增/替换一个模型 provider。
    ///
    /// 让"设置页新增服务商"**立即生效**，不必重启进程 ——
    /// 之前注册表在启动时建好就不可变，用户改完只能重启，而提示里写
    /// "重启后生效"既没说清重启什么，也让一台正在跑的会话没法用上新服务商。
    pub fn add_model(
        &mut self,
        info: crate::models::ModelInfo,
        p: Box<dyn ModelProvider>,
    ) -> Result<(), String> {
        self.models.add(info, p)
    }

    /// 运行时移除一个模型 provider（不允许移除当前在用的）。
    pub fn remove_model(&mut self, name: &str) -> Result<bool, String> {
        self.models.remove(name)
    }

    /// 全部可选模型（宿主列表用）。
    pub fn available_models(&self) -> Vec<crate::models::ModelInfo> {
        self.models.list()
    }

    /// 当前模型的上下文窗口（0 = 未知）。
    pub fn current_context_limit(&self) -> u64 {
        self.models.current_context_limit()
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

    /// 把 `@file` / `$skill` 引用解析成一段可注入的上下文块。
    ///
    /// **只解析这两类**，其余按开放语义不产生块：
    /// - `@file`：读文件（经沙箱），支持 `#行号` 范围；读不到则如实记一条失败。
    /// - `$skill`：查注册表；找到则注入正文，找不到则把**可用技能名**列出来
    ///   （只报"找不到"会让用户面对空注册表而不知道下一步）。
    /// - `#session` / `/command`：**刻意不注入为文本**。它们是"去某个宿主动作"
    ///   而不是"给模型的内容"。早期把 `/compact` 也塞进文本，
    ///   模型会把它当成一个要自己处理的词 —— 命令该由宿主解析执行。
    ///
    /// 每条引用的字节上限用 `max_output_bytes`（与工具输出同一把尺），
    /// 超限置 `truncated` 标记 —— 内存有界是内核义务，引用不能开后门。
    fn resolve_refs(&self, refs: &[ContextRef]) -> RefResolution {
        let mut summary: Vec<String> = Vec::new();
        let mut sections: Vec<String> = Vec::new();

        for r in refs {
            match r.kind {
                RefKind::File => {
                    match self.sandbox.execute(
                        self.resolution().sandbox,
                        &format!("cat -- {}", shell_quote(&r.target)),
                        self.max_output_bytes,
                    ) {
                        SandboxOutcome::Ran { stdout, truncated } => {
                            let (body, cut) = truncate_lines(&stdout, r.lines, self.max_output_bytes);
                            let cut = cut || truncated;
                            let range = match r.lines {
                                Some((a, b)) => format!(":{a}-{b}"),
                                None => String::new(),
                            };
                            summary.push(format!(
                                "📄 {}{}{} 已注入",
                                r.target,
                                range,
                                if cut { " [截断]" } else { "" }
                            ));
                            sections.push(format!(
                                "<file path=\"{}{}\"{}>\n{}\n</file>",
                                r.target,
                                range,
                                if cut { " truncated=\"true\"" } else { "" },
                                body
                            ));
                        }
                        SandboxOutcome::Denied { reason } => {
                            summary.push(format!("📄 {} 读取被拒：{reason}", r.target));
                        }
                    }
                }
                RefKind::Skill => match self.skills.get(&r.target) {
                    Some(s) => {
                        summary.push(format!("🧩 ${} 已注入（{} 字节）", s.name, s.body.len()));
                        sections.push(format!(
                            "<skill name=\"{}\">\n{}\n</skill>",
                            s.name, s.body
                        ));
                    }
                    None => {
                        let avail = self.skills.names();
                        let hint = if avail.is_empty() {
                            "（当前未加载任何技能）".to_string()
                        } else {
                            format!("（可用：{}）", avail.join(", "))
                        };
                        summary.push(format!("🧩 ${} 未找到 {hint}", r.target));
                    }
                },
                RefKind::Session | RefKind::Command => {
                    // 宿主负责解释这两类，内核不把它们当模型上下文注入。
                }
            }
        }

        let block = if sections.is_empty() {
            None
        } else {
            Some(format!("[上下文引用]\n{}", sections.join("\n\n")))
        };
        RefResolution { summary, block }
    }

    /// 真正执行一个调用：经沙箱、落日志、进历史。
    fn execute_one(&mut self, call: &ToolInvocation) -> Result<(), KernelError> {
        // 改动预览必须在**执行前**取。执行后文件内容已等于目标，
        // `preview` 会返回 None（"没有改动"），统计就永远为空 ——
        // 这个顺序错误只会在真实工具上暴露：假工具的 preview 是无条件返回的。
        let change_before = self
            .tools
            .get(&call.name)
            .and_then(|t| t.preview(&call.arguments));

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
        // 工具声明的"运行后公告"（如任务清单更新）先于结束事件发出 ——
        // 顺序对宿主有意义：先看到清单变化，再看到调用结束。
        if let Some(tool) = self.tools.get(&call.name) {
            for ev in tool.report(&call.arguments) {
                self.emit_and_log(&ev)?;
            }
        }

        // 文件改动统计：用**执行前**取到的预览（见上方 change_before）
        if output.exit_code == 0 {
            if let Some((path, diff)) = change_before {
                let additions = diff
                    .lines()
                    .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
                    .count();
                let deletions = diff
                    .lines()
                    .filter(|l| l.starts_with('-') && !l.starts_with("---"))
                    .count();
                if additions > 0 || deletions > 0 {
                    let e = self.file_changes.entry(path).or_insert((0, 0));
                    e.0 += additions;
                    e.1 += deletions;
                    let files: Vec<FileChange> = self
                        .file_changes
                        .iter()
                        .map(|(p, (a, d))| FileChange {
                            path: p.clone(),
                            additions: *a,
                            deletions: *d,
                        })
                        .collect();
                    let ev = EventMsg::FilesChanged { files };
                    self.emit_and_log(&ev)?;
                }
            }
        }

        let ev = EventMsg::ToolCallEnd {
            id: call.id.clone(),
            exit_code: output.exit_code,
            // 输出必须进事件流：用户要看的是"这条命令打印了什么"，
            // 不是一个孤零零的退出码。内核已按上限截断。
            stdout: output.stdout.clone(),
            stderr: output.stderr.clone(),
            truncated: output.truncated,
        };
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

    /// 把项目指令（系统提示词的组成）落一次日志。
    ///
    /// 空指令不落 —— 发一条 `sources: []` 的事件只会让日志多一行噪音，
    /// 而"没有指令"本就是默认状态。
    fn ensure_instructions_logged(&mut self) -> Result<(), KernelError> {
        if self.instructions_logged || self.instructions.is_empty() {
            return Ok(());
        }
        let ev = EventMsg::InstructionsLoaded {
            sources: self.instructions.sources.clone(),
            block: self.instructions.block.clone(),
            truncated: self.instructions.truncated,
        };
        self.emit_and_log(&ev)?;
        self.instructions_logged = true;
        Ok(())
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

/// 单步推进的结果。与 `ExecOutcome` 分开：一个是"这一步的工具执行完了吗"，
/// 一个是"整轮推进到哪了"。
enum StepOutcome { More, Suspended, Done }

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
