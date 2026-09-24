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
pub mod agents;
pub mod skills;
pub mod instructions;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// 一步内允许的最大工具调用数（防御失控模型）。
pub const MAX_TOOL_CALLS_PER_STEP: usize = 32;

/// 连续**完全相同**的工具调用达到该次数时，在上下文里提醒模型（护栏 #1）。
/// 对齐 ZCode 的 `detectRepeatedToolCallWarnings`（第 3 次相同即提醒）。
pub const REPEATED_CALL_WARN_AT: usize = 3;
/// 单轮内护栏提醒最多注入次数 —— 防止模型刷屏式地触发提醒又忽略。
pub const MAX_GUARD_WARNINGS_PER_TURN: usize = 3;
/// 单轮工具调用数达到该阈值时提醒模型收尾（护栏 #2，**低于**熔断）。
pub const TOOL_CALL_BUDGET_WARN_PER_TURN: usize = 10;
/// 单轮工具调用数熔断。超过即停止本轮 —— 不是"工作限额"，是失控刹车。
/// 提醒阈值刻意低于它，留一档让模型自我纠正（护栏 #2）。
pub const TOOL_CALL_FUSE_PER_TURN: usize = 32;

/// 会话 token 预算达到该百分比时，向模型与用户各提醒一次（护栏 #9）。
pub const SESSION_BUDGET_WARN_PERCENT: u64 = 70;

/// 只读工具并行的并发上限（护栏 #8）。对齐 ZCode `maxConcurrency: 10`。
/// 超过分批（12 个 read → 10 + 2）；**写操作永不在并行组里**。
pub const MAX_TOOL_CONCURRENCY: usize = 10;

/// 连续相同调用触发 **doom-loop 独立闸门** 的次数（护栏 #6）。
/// 第 3 次已有上下文提醒（#1）；第 4 次起必须问人 —— **最高权限档也不静默放行**。
pub const DOOM_LOOP_GATE_AT: usize = 4;
/// 一轮内允许的最大步数。超出即报错结束该轮，不无限循环。
/// 单轮最大步数（一步 = 一次模型请求 + 它要求的工具执行）。
///
/// 这是**安全阀**（防死循环），不是"工作限额"。曾经默认 16 —— 真实任务
/// 里一次"了解下这个项目"就能把预算用完（用户实际撞到"超出步数预算（16 步）"），
/// 表现为干到一半被硬停。调到 64：仍能阻止失控循环，但不至于拦下正常任务。
pub const DEFAULT_MAX_STEPS: usize = 64;

/// 上下文消息上限。
///
/// 达到上限时内核**报错并要求压缩**，而不是静默丢消息 ——
/// 静默丢会让模型的视角与日志不一致，破坏"模型可见即已落盘"的铁律。
/// 压缩本身是 L4 orchestration 的职责（`Compact` Op），当前**未实现**。
pub const DEFAULT_MAX_CONTEXT_MESSAGES: usize = 4096;

/// 一次 `Op::Pump` 消费流式增量的时间片。
///
/// 逐帧宿主按 Pump 重绘：切片让重绘频率有界（约 10 帧/秒），
/// 又不至于整步等完才返回 —— 那会让思考型模型的长推理期间界面
/// 全程"运行中"却看不到任何内容（真实反馈）。
/// 切片只影响**同一事件序列的分段方式**，不改变事件本身（T2 不受影响）。
const PUMP_SLICE: std::time::Duration = std::time::Duration::from_millis(80);
/// 单次 Pump 消费的增量上限（内存与日志有界；输出爆炸的兜底）。
pub const MAX_DELTAS_PER_PUMP: usize = 256;

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
    /// 带图的用户轮（`view_image` 注入）。provider 编成 OpenAI
    /// `image_url` content parts；非视觉模型会忽略图或只看 text。
    ///
    /// 与 `User(String)` 分开而不是塞进 ToolResult：OpenAI 兼容接口的
    /// `role:"tool"` 通常只收字符串，图片必须挂在 user/assistant content 数组上。
    UserImage {
        text: String,
        image: neo_protocol::ImageAttachment,
    },
}

// ══════════════════════════════════════════════════════════════════════
// 扩展点 1/4：ModelProvider（流式 + 支持工具调用）
// ══════════════════════════════════════════════════════════════════════

/// 工具对模型的呈现（只含模型需要知道的字段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    /// 参数 JSON Schema（原样给 provider 的 function.parameters）
    pub parameters: Value,
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
    /// 模型的**推理过程**（DeepSeek 的 `reasoning_content`）。
    ///
    /// # 为什么必须在这一层就有它
    ///
    /// 协议层早就有 `EventMsg::ReasoningDelta` 与 `Fact::AssistantThought`，
    /// TUI 也早就会渲染"思考过程"，但 **`ModelDelta` 里没有对应的变体** ——
    /// provider 没有任何途径把推理报上来。于是整条链路是断的：
    /// 用户"看不到思考过程"不是显示问题，是**第一个环节就不存在**。
    /// 又一个"两端都写好了、中间没接"。
    Reasoning(String),
    ToolCall(ToolInvocation),
    Usage { input_tokens: u64, output_tokens: u64 },
}

/// 一次模型响应的增量序列。
///
/// 用迭代器而非 `Vec` 是为了**保留流式语义**：真实 provider（SSE）边收边
/// yield，内核也边收边发 `AgentMessageDelta` / `ReasoningDelta`。
/// 迭代器由内核**跨 Op 存活**（见 `Kernel::in_flight`）——
/// 一次 `Op::Pump` 只消费一个时间片，宿主据此逐帧看到思考与正文。
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
// 扩展点：GoalOrchestrator（目标编排）
// ══════════════════════════════════════════════════════════════════════
//
// 与 Compactor 同一条理由：目标怎么拆、阶段怎么走是 **L4 的策略**，
// 内核只提供机制 —— 把编排器产出的事件落日志、把子任务轮当普通 turn 跑
// （沙箱/审批/上限全部复用），以及在回放时把日志喂回去重建状态。

/// 目标编排契据。实现方维护全部编排状态；内核不知道"阶段"长什么样。
///
/// # 有界性由实现方负责
///
/// `next_turn_prompt` 每消费一次就少一轮；停止条件触发后
/// `has_pending_turn` 必须返回 false —— 否则宿主的推进循环没有出口。
///
/// # 回放即重建
///
/// `observe` 会被内核在 `rebuild_from_log` 时对**每一条**事件调用：
/// 实现方从自己发出过的快照事件里恢复状态（kill 进程后续跑的核心）。
pub trait GoalOrchestrator: Send {
    /// 当前目标 id；无活动目标为 `None`。
    fn goal_id(&self) -> Option<String>;
    /// 当前完整快照（`thread/goal/get` 只读查询）。无活动目标为 `None`。
    fn snapshot(&self) -> Option<neo_protocol::GoalSnapshot>;
    /// 设定（或重定向）目标。返回要落日志的事件（快照）。
    fn set_goal(&mut self, goal: &str) -> Vec<EventMsg>;
    /// 暂停：目标保留，停止推进。
    fn pause(&mut self) -> Vec<EventMsg>;
    /// 恢复推进。
    fn resume(&mut self) -> Vec<EventMsg>;
    /// 清除目标（发 `GoalCleared`；之后 `goal_id` 为 None）。
    fn clear(&mut self) -> Vec<EventMsg>;
    /// 是否还有待执行的子任务轮（暂停/停止/无目标 = false）。
    fn has_pending_turn(&self) -> bool;
    /// 取下一轮的提示词（消费一轮额度）。`None` = 没有待执行轮。
    fn next_turn_prompt(&mut self) -> Option<String>;
    /// 一个子任务轮结束。`usage` = 本轮 token 消耗，`failed` = 本轮的
    /// 硬失败信号（Error 事件 / 非零退出的工具调用），`review_text` =
    /// 本轮**模型的最终答复**（审查阶段由实现方解析显式结论）。
    /// 实现方据此推进阶段/重试/判停，返回状态快照事件。
    fn on_turn_complete(
        &mut self,
        usage: (u64, u64),
        failed: bool,
        review_text: &str,
    ) -> Vec<EventMsg>;
    /// 回放：消费日志事件、重建内部状态。必须幂等且与在线推进一致。
    fn observe(&mut self, event: &EventMsg);
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

/// 基础系统提示词（不含工具清单与项目指令）。
///
/// # 那句"简单问题直接回答"为什么必须存在
///
/// 之前只有"优先用工具核验事实，不要凭记忆断言"。对**需要核验仓库事实**的
/// 任务这是对的；但模型会把它推广到一切问题 —— 于是问"你是谁"它也先去跑
/// 工具探查，**弹出工具审批**、还要用户授权（真实反馈），既慢又莫名其妙。
///
/// 提示词要同时说清**什么时候该用工具**和**什么时候不该**，只写一半
/// 会让模型过度使用（这也是把行为写成"单向偏好"的通病）。
pub fn base_system_prompt() -> String {
    String::from(
        "你是 NEO 的编码 agent。\n\
\n\
关于是否使用工具：\n\
- 需要核验仓库事实（读代码、找文件、跑命令、改文件）时，**必须用工具**，\
不要凭记忆断言。\n\
- 问候、身份、自我介绍、概念解释这类**不依赖仓库现状**的问题，\
**直接用已有信息回答，不要为它们调用工具**。用户问「你是谁」时\
直接说清自己的定位即可。\n\
- 不确定要不要用工具时，先判断「答案是否取决于当前仓库的内容」。\n\
\n可用工具：\n",
    )
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
    ///
    /// # 为什么必须接收 `cwd`
    ///
    /// 预览与执行**必须解析到同一个文件**：用户是照着预览批准改动的，
    /// 若预览读的是 A 文件、执行写的是 B 文件，那这份"不盲批"的保证就是假的。
    ///
    /// 曾经这里没有 `cwd`，实现只能按**进程当前目录**读相对路径 —— 而
    /// `ToolCtx::resolve` 是按 `cwd`（工作区）解析的。两者在
    /// `--workspace <非进程目录>` 时必然不同：真机复现过一次 —— 工作区是
    /// `/tmp/x`、进程 CWD 是仓库根，仓库根恰好有个同名文件且内容相同，
    /// 于是预览算出"空 diff"、界面**什么都不显示**，而用户仍被要求批准一次
    /// 真实写入（正是 preview 要消灭的"盲批"）。
    fn preview(&self, _args: &Value, _cwd: &std::path::Path) -> Option<(String, String)> {
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
    /// 执行成功后要交给模型的图片（`view_image`）。
    ///
    /// 默认 `None`。与 `execute` 分开：base64 不进 `ToolOutput`（避免改
    /// 全仓字面量），也不进事件流（避免撑爆 JSONL）——只在内存 `Message` 里。
    fn image_result(
        &self,
        _args: &Value,
        _output: &ToolOutput,
    ) -> Option<neo_protocol::ImageAttachment> {
        None
    }
    fn execute(&self, args: &Value, ctx: &ToolCtx) -> ToolOutput;

    /// 参数的 JSON Schema（OpenAI function-calling 的 `parameters`）。
    ///
    /// 为什么必须有：真机回归（glm-4.6）发现,没有结构化 schema 时模型
    /// 只能靠散文描述猜参数形状,曾把整个数组**字符串化**后传进来
    /// （双重编码）,反复重试浪费大量预算。schema 是模型与工具之间
    /// 唯一可靠的结构契约。默认空对象 schema（任意参数）——
    /// 没有结构化参数的工具不必实现。
    fn parameters(&self) -> Value {
        serde_json::json!({"type": "object", "properties": {}})
    }
}

/// 用 BTreeMap 而非 HashMap —— 保证工具描述顺序稳定（提示词缓存命中的前提）
/// `Clone` 供子代理工厂做白名单裁剪(Arc 内层,克隆廉价)。
#[derive(Clone)]
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
            .map(|t| ToolSchema {
                name: t.name().into(),
                description: t.describe(),
                parameters: t.parameters(),
            })
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

/// 调用签名：工具名 + 参数稳定序列化（护栏 #1 / #6 共用）。
fn call_signature(call: &ToolInvocation) -> String {
    format!("{}|{}", call.name, serde_json::to_string(&call.arguments).unwrap_or_default())
}

/// 参数里的第一个字符串值的摘要(≤40 字符) —— 审批 detail 的动作部分。
/// 内核不解析工具特有参数名(那是工具的知识),只取"第一个字符串"作示意:
/// apply_patch → path,bash → cmd,覆盖常见形态。
fn call_arg_summary(args: &serde_json::Value) -> String {
    let Some(obj) = args.as_object() else { return String::new() };
    for v in obj.values() {
        if let Some(s) = v.as_str() {
            let head: String = s.chars().take(40).collect();
            let ell = if s.chars().count() > 40 { "…" } else { "" };
            return format!("{head}{ell}");
        }
    }
    String::new()
}

/// 类别的协议名（`EventMsg::ApprovalRequest.kind`）。
/// 审批卡片上"总是允许"将放行的范围由此单一事实源给出 ——
/// 宿主按工具名自行推断会与内核的实际放行范围分叉。
fn call_kind_name(kind: CallKind) -> &'static str {
    match kind {
        CallKind::Read => "read",
        CallKind::Write => "write",
        CallKind::Network => "network",
        CallKind::Interactive => "interactive",
    }
}

// ══════════════════════════════════════════════════════════════════════
// 内核：turn/step 主循环
// ══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelError {
    /// 没有待审批的调用却收到 Approve / RespondUserInput。
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
    /// 会话 token 预算已用尽（护栏 #9）：拒绝开启新轮，已完成的工作保留。
    ///
    /// 与 `ContextBudgetExceeded` 分开：一个说"历史太长要压缩"，
    /// 一个说"预算用完要提额或开新会"——混用会让用户去点错按钮。
    SessionTokenBudgetExceeded { used: u64, limit: u64 },
    /// Goal 系列无法执行：没配置编排策略，或目标已暂停/停止/无待执行轮。
    ///
    /// 单独变体：让 "没配策略" 与 "目标此刻不可推进" 有各自的措辞，
    /// 用户才知道下一步是"换宿主配置"还是"先 /goal resume"。
    GoalUnavailable(String),
    /// 在飞模型流还没消费完就提交了不合法的 Op。
    ///
    /// 流式进行中只允许 `Pump`（继续消费）与 `Interrupt`（作废整步）；
    /// 其余 op 会与在飞步的历史操作交错，破坏"一步 = 一次模型请求 + 其工具"的原子性。
    TurnInFlight,
}

impl std::fmt::Display for KernelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPendingApproval(id) => write!(f, "无待处理的挂起（审批/用户输入）：{id}"),
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
            Self::SessionTokenBudgetExceeded { used, limit } => write!(
                f,
                "会话 token 预算已用尽（{used} ≥ {limit}），已停止以保护预算；提高预算或开新会话后再继续"
            ),
            Self::GoalUnavailable(msg) => write!(f, "目标编排不可用：{msg}"),
            Self::TurnInFlight => write!(
                f,
                "本轮仍在流式推进中：等待 Pump 到边界，或 Interrupt 中断"
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
    /// `request_user_input` 挂起：等宿主 `Op::RespondUserInput`。
    AwaitingUserInput { id: ApprovalId },
}

/// 挂起点：同一步内，第 `index` 个调用等待审批。
///
/// 保存**整批调用**而非单个，是为了恢复后能继续执行同一步剩余调用 ——
/// 否则模型会看到"半个步骤"，工具结果顺序也会错乱。
#[derive(Debug, Clone)]
struct PendingApproval {
    calls: Vec<ToolInvocation>,
    index: usize,
    /// 本次挂起是否为 doom-loop 独立闸门（护栏 #6）。
    /// 循环闸门的「总是允许」**不得**写入 `granted`（否则等于永久关闸）。
    loop_gate: bool,
    /// true = 等用户自由文本（`RespondUserInput`），false = 等审批（`Approve*`）。
    user_input: bool,
}

/// 一步中尚未消费完的模型流（跨 `Op::Pump` 存活）。
///
/// 流存内核而非步骤局部变量，是"逐帧"的前提：一次 Pump 只消费一个
/// 时间片的增量就返回，宿主立即重绘 —— 思考文本到一笔显一笔。
/// 宿主拿到的事件序列与整步消费**完全一致**，切片只是分段方式。
struct InFlightStep {
    stream: ModelStream,
    text: String,
    calls: Vec<ToolInvocation>,
}

pub struct Kernel {
    cfg: Config,
    tools: ToolRegistry,
    /// 模型注册表（多 provider + 当前选中）。运行时可切换。
    models: crate::models::ModelRegistry,
    /// 上下文压缩策略（L4 实现，构造时注入；缺省不压缩）
    compactor: Box<dyn Compactor>,
    /// 目标编排器（L4 实现，构造时注入；缺省 = Goal 系列如实报"未配置"）
    goal: Option<Box<dyn GoalOrchestrator>>,
    /// 当前是否有一轮**目标子任务轮**在飞（从 GoalAdvance 开始，
    /// 到 TurnComplete 结束；审批挂起期间保持）。
    goal_turn_in_flight: bool,
    /// 目标子任务轮内是否发生过 Error 事件（引擎的审查失败判据）
    goal_turn_failed: bool,
    /// 目标子任务轮内模型最后一条答复（审查阶段解析显式结论用）
    goal_turn_last_text: Option<String>,
    /// 目标子任务轮内各调用的语义类别（id → kind）——
    /// 审查记账用它区分"探测失败"与"修复失败"
    goal_turn_call_kinds: std::collections::BTreeMap<String, CallKind>,
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
    ///
    /// 见 `base_system_prompt()` —— 里面那句"简单问题直接回答"不是客套：
    /// 缺了它，模型会对"你是谁"这类问题也先跑工具探查，然后弹出工具审批
    /// （真实反馈），用户莫名其妙。
    system_prompt: String,
    /// 工具 schema：构造时算一次，同样避免每步分配。
    tool_schemas: Vec<ToolSchema>,

    session_id: String,
    state: KernelState,
    /// 模型可见历史（派生物；真相在日志）
    messages: Vec<Message>,
    steps_this_turn: usize,
    /// 本轮已执行的工具调用数（护栏 #2：预算提醒 / 熔断）。
    calls_this_turn: usize,
    /// 本轮已注入的护栏提醒次数（防刷屏）。
    guard_warnings_this_turn: usize,
    /// 上一次工具调用签名（工具名 + 稳定序列化参数），护栏 #1 重复检测用。
    last_call_sig: Option<String>,
    /// 与 `last_call_sig` 相同的连续次数。
    repeat_count: usize,
    /// 会话级 token 预算（护栏 #9）。`None` = 未设限。
    session_token_budget: Option<u64>,
    /// 会话累计已用 token（跨轮，不随 `begin_turn` 清零）。
    session_tokens_used: u64,
    /// 接近预算的提醒是否已注入（设新预算时重置）。
    session_budget_warned: bool,
    turn_counter: u64,
    step_counter: u64,
    /// 本轮是否仍在进行（`begin_turn` 置真，`TurnComplete` 置假）。
    /// `turn/steer` 用它判断能否注入 —— 不靠 `steps_this_turn>0`
    /// （收轮后该计数不会清零，会误判轮仍在飞）。
    turn_active: bool,
    /// 用户直输 shell 命令的序号（`Op::Shell`）。
    ///
    /// 单独一个计数器，因为它在**任何轮次之外**：`!ls` 不发 turn、也不驱动模型
    /// （见 `Op::Shell` 的处理）。曾用 `turn_counter` 当 id 的一部分，
    /// 而 shell 路径不递增它 —— 连续两条 `!cmd` 会拿到**同一个 id**
    /// （都是 `shell-0`），而 id 的用途正是标识一次调用（工具结果按 id 配对）。
    shell_counter: u64,
    usage_in: u64,
    usage_out: u64,
    pending: Option<PendingApproval>,
    /// 在飞的模型流（`Op::Pump` 分帧消费；整步结束后为 None）。
    in_flight: Option<InFlightStep>,
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
        let mut system_prompt = base_system_prompt();
        system_prompt.push_str(&tools.render_prompt());

        Self {
            cfg,
            tools,
            models: model,
            compactor: Box::new(NoCompactor),
            goal: None,
            goal_turn_in_flight: false,
            goal_turn_failed: false,
            goal_turn_last_text: None,
            goal_turn_call_kinds: Default::default(),
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
            calls_this_turn: 0,
            guard_warnings_this_turn: 0,
            last_call_sig: None,
            repeat_count: 0,
            session_token_budget: None,
            session_tokens_used: 0,
            session_budget_warned: false,
            turn_counter: 0,
            step_counter: 0,
            turn_active: false,
            shell_counter: 0,
            usage_in: 0,
            usage_out: 0,
            pending: None,
            in_flight: None,
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
        let mut p = base_system_prompt();
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
    /// 工具目录（给客户端的 tools/list）。顺序与 prompt 渲染一致（稳定）。
    pub fn tool_schemas(&self) -> Vec<ToolSchema> {
        self.tools.schemas()
    }

    pub fn session_id(&self) -> &str { &self.session_id }

    /// 内核工作区（工具解析相对路径的基准；git/info 缺省也用它）。
    pub fn cwd(&self) -> &std::path::Path {
        &self.cwd
    }

    /// 当前生效的执行语义（三维）。
    ///
    /// **以 `exec_mode` 为唯一事实源**：五档模式是 UI 的唯一入口，避免了
    /// 「双轴与档位不一致」这一经典困惑（见 ADR-0004）。`Config` 上的
    /// `sandbox_mode`/`approval_policy` 字段不参与内核对轴的决定。
    pub fn resolution(&self) -> ModeResolution { resolve(self.cfg.exec_mode) }

    /// 当前执行模式（Codex `config/read` 用）。
    pub fn exec_mode(&self) -> neo_protocol::ExecMode {
        self.cfg.exec_mode
    }

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
        // 流式进行中只接受 Pump/Interrupt：其余 op 会与在飞步的消息/工具
        // 操作交错（历史被改、审批凭空出现），必须整步完成后才可提交。
        if self.in_flight.is_some() && !matches!(op, Op::Pump | Op::Interrupt) {
            return Err(KernelError::TurnInFlight);
        }
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
                //
                // id 用**自己的**计数器：shell 在任何轮次之外，用 `turn_counter`
                // 会让连续两条命令拿到同一个 id（那个计数器在这里不递增）。
                let id = format!("shell-{}", self.shell_counter);
                self.shell_counter += 1;
                let call = ToolInvocation {
                    id,
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

            Op::Approve { id, decision, reason } => {
                self.resolve_approval(id, decision, reason, true)?;
            }
            // 同 `Approve`，但**不驱动**后续步骤（逐帧宿主用，配合 `Op::Pump`）。
            Op::ApproveStep { id, decision, reason } => {
                self.resolve_approval(id, decision, reason, false)?;
            }
            Op::RespondUserInput { id, response } => {
                self.resolve_user_input(id, response)?;
            }

            Op::Rewind { turns } => {
                // 找到倒数第 `turns` 个用户消息的下标，从那里截断。
                // 用"用户消息"当轮次边界，而不是"助手消息"—— 一轮可能包含
                // 多个助手消息（工具调用），只有用户消息才是轮的起点。
                let mut seen = 0usize;
                let mut cut = None;
                for (i, m) in self.messages.iter().enumerate().rev() {
                    if matches!(m, Message::User(_) | Message::UserImage { .. }) {
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
                // 被回退掉的目标子任务轮同理作废（轮都没了，谈不上完成）
                self.goal_turn_in_flight = false;
                self.goal_turn_failed = false;
                self.goal_turn_last_text = None;
                self.goal_turn_call_kinds.clear();
                let ev = EventMsg::Rewound {
                    turns,
                    removed_messages: removed,
                    files_kept: self.file_changes.len(),
                };
                self.emit_and_log(&ev)?;
            }

            Op::Interrupt => {
                // 中断只在**安全点**生效，杜绝半写状态：工具调用边界，或
                // 流式增量的分片边界。在飞流直接作废 —— 增量尚未落成
                // 消息/工具调用，历史不会被污染；SSE 连接随迭代器 Drop
                // 一起终止（openssl 子进程被 kill，不留残留）。
                self.in_flight = None;
                // 审批挂起中的剩余调用必须补占位（护栏 #4）：Assistant 已带
                // 全部 tool_calls，只补到 index 会让后面的 id 永远悬空，
                // 下一轮 chat-completions 请求会被 API 拒绝。
                if let Some(pending) = self.pending.take() {
                    self.push_cancelled_results(
                        &pending.calls,
                        pending.index,
                        "用户中断，本调用未执行",
                    );
                }
                self.state = KernelState::Idle;
                // 被中断的目标子任务轮不再有 TurnComplete —— 必须同步作废
                // 在飞标记，否则下一个普通轮的结束会被误当成目标轮的结束。
                self.goal_turn_in_flight = false;
                self.goal_turn_failed = false;
                self.goal_turn_last_text = None;
                self.goal_turn_call_kinds.clear();
                self.turn_active = false;
                self.emit_and_log(&EventMsg::Error { message: "已中断".into() })?;
            }

            Op::Steer { text } => {
                // Codex turn/steer：当前轮注入，不新开轮。
                if self.in_flight.is_some() {
                    return Err(KernelError::TurnInFlight);
                }
                if !matches!(self.state, KernelState::Idle) {
                    return Err(KernelError::GoalUnavailable(
                        "有未决审批/输入时不能 steer，先 respond".into(),
                    ));
                }
                if !self.turn_active {
                    return Err(KernelError::GoalUnavailable(
                        "当前没有进行中的 turn（steer 只能在轮内注入）".into(),
                    ));
                }
                let text = text.trim().to_string();
                if text.is_empty() {
                    return Err(KernelError::GoalUnavailable("steer 文本为空".into()));
                }
                let ev = EventMsg::UserSubmitted { text: text.clone() };
                self.emit_and_log(&ev)?;
                self.messages.push(Message::User(text));
                self.drive_steps()?;
            }

            Op::ConfigureSession { patch } => {
                // 会话预算在 merge 消费 patch 之前取出（0 = 清除，见 SessionPatch 文档）
                if let Some(b) = patch.token_budget {
                    self.set_session_token_budget(b);
                }
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

            // ── 目标编排（Goal）────────────────────────────────────
            // 机制在内核（落日志、跑子任务轮、回放重建），策略在编排器。
            Op::GoalSet { goal } => {
                let events = self.with_goal(|g| g.set_goal(&goal))?;
                for ev in &events {
                    self.emit_and_log(ev)?;
                }
            }
            Op::GoalPause { goal_id } => {
                self.check_goal_id(&goal_id)?;
                let events = self.with_goal(|g| g.pause())?;
                for ev in &events {
                    self.emit_and_log(ev)?;
                }
            }
            Op::GoalResume { goal_id } => {
                self.check_goal_id(&goal_id)?;
                let events = self.with_goal(|g| g.resume())?;
                for ev in &events {
                    self.emit_and_log(ev)?;
                }
            }
            Op::GoalClear => {
                let events = self.with_goal(|g| g.clear())?;
                for ev in &events {
                    self.emit_and_log(ev)?;
                }
            }
            Op::GoalAdvance => {
                // 取一个待执行的子任务轮，按普通 turn 跑 —— 沙箱、审批、
                // 上限、落盘全部复用既有链路，不为目标另开执行通道。
                // begin_turn 只开场不驱动：逐帧宿主随后 Pump（与 UserTurn
                // 同一个不冻结的架构）。
                let prompt = {
                    let g = self.goal.as_mut().ok_or_else(|| {
                        KernelError::GoalUnavailable("未配置编排策略".into())
                    })?;
                    if !g.has_pending_turn() {
                        return Err(KernelError::GoalUnavailable(
                            "没有待执行的子任务轮（可能已暂停、停止或全部完成）".into(),
                        ));
                    }
                    g.next_turn_prompt()
                        .ok_or_else(|| KernelError::GoalUnavailable("没有待执行的子任务轮".into()))?
                };
                self.goal_turn_in_flight = true;
                self.goal_turn_failed = false;
                self.goal_turn_last_text = None;
                self.goal_turn_call_kinds.clear();
                self.begin_turn(prompt, Vec::new())?;
                self.drive_steps()?;
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
                // 护栏 #9：预算已用尽则拒绝开新轮（已完成的工作保留在历史里）
                self.check_session_token_budget()?;
                // 上一轮末可能没跨过 70% 线、但本周期望看到提醒；begin 再兜一次
                self.maybe_warn_session_budget()?;
                self.turn_counter += 1;
                self.steps_this_turn = 0;
                self.turn_active = true;
                self.calls_this_turn = 0;
                self.guard_warnings_this_turn = 0;
                self.last_call_sig = None;
                self.repeat_count = 0;
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
    /// 拆出来是为了让宿主能逐帧推进（每帧重绘），而不是等整轮结束 ——
    /// 整轮可能包含多次网络往返，期间界面完全冻结（真实反馈："像卡死"）。
    /// 帧的粒度是**时间片**而非整步：有在飞流时只消费一个切片的增量就返回，
    /// 流耗尽才落消息、执行工具 —— 思考型模型的长推理期间宿主也能持续重绘，
    /// 而不是全程"运行中"到最后一次性出结果（真实反馈）。
    ///
    /// 确定性不受影响：事件序列只由 provider 的产出顺序决定，
    /// 时间片只是**同一序列的分段方式**（T2 断言的是序列，不是分段）。
    fn step_once(&mut self) -> Result<StepOutcome, KernelError> {
        if self.in_flight.is_some() {
            return self.consume_stream_slice();
        }
        if self.steps_this_turn >= self.max_steps {
            let msg = EventMsg::Error {
                message: format!(
                    "本轮已达步数上限（{} 步），已停止以免失控。\
                     已完成的工作都在上面；继续的话再发一条消息接着做，\
                     或用 --max-steps 提高上限。",
                    self.max_steps
                ),
            };
            self.emit_and_log(&msg)?;
            return Ok(StepOutcome::Done);
        }
        self.check_context_budget()?;
        self.steps_this_turn += 1;
        self.step_counter += 1;

        // 开流：历史临时移出组装请求（零拷贝 —— 每步克隆整份历史会让一轮
        // 退化到 O(N²)，长会话下是主要热点）。请求体被迭代器自持后立即放回，
        // 后续 Pump 只消费流，不再需要历史。
        let stream = {
            let messages = std::mem::take(&mut self.messages);
            let request = ModelRequest {
                system: &self.system_prompt,
                messages: &messages,
                tools: &self.tool_schemas,
            };
            let stream = self.models.current_provider().stream(&request);
            self.messages = messages;
            stream
        };
        self.in_flight = Some(InFlightStep { stream, text: String::new(), calls: Vec::new() });
        self.consume_stream_slice()
    }

    /// 消费在飞流的一个时间片；流耗尽时收尾这一步（落消息 / 执行工具）。
    ///
    /// 事件在增量到达时**立即**进 outbox：宿主下一次 Pump 返回就能看到，
    /// 思考与正文因此逐帧生长。流半途被丢弃（中断/落盘失败）时，
    /// SSE 连接随迭代器 Drop 一起终止，不留残留进程。
    fn consume_stream_slice(&mut self) -> Result<StepOutcome, KernelError> {
        let Some(mut inf) = self.in_flight.take() else {
            return Ok(StepOutcome::Done);
        };
        let deadline = std::time::Instant::now() + PUMP_SLICE;
        let mut consumed = 0usize;
        let mut finished = false;
        while consumed < MAX_DELTAS_PER_PUMP {
            match inf.stream.next() {
                None => {
                    finished = true;
                    break;
                }
                Some(ModelDelta::Text(chunk)) => {
                    inf.text.push_str(&chunk);
                    self.emit_and_log(&EventMsg::AgentMessageDelta { delta: chunk })?;
                    consumed += 1;
                }
                Some(ModelDelta::Reasoning(chunk)) => {
                    // 推理也要落盘：它同样是**模型可见内容**（下一轮请求
                    // 会带上 assistant 的 reasoning），不落日志回放就缺一块。
                    self.emit_and_log(&EventMsg::ReasoningDelta { delta: chunk })?;
                    consumed += 1;
                }
                Some(ModelDelta::ToolCall(call)) => {
                    let ev = EventMsg::ToolCallBegin {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    };
                    self.emit_and_log(&ev)?;
                    inf.calls.push(call);
                    consumed += 1;
                }
                Some(ModelDelta::Usage { input_tokens, output_tokens }) => {
                    self.usage_in += input_tokens;
                    self.usage_out += output_tokens;
                    consumed += 1;
                }
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
        }
        if !finished {
            self.in_flight = Some(inf);
            return Ok(StepOutcome::More);
        }
        // 流耗尽：收尾与整步消费完全一致（AgentMessageDone → 落消息 → 工具）
        self.emit_and_log(&EventMsg::AgentMessageDone { text: inf.text.clone() })?;
        self.messages.push(Message::Assistant { text: inf.text, tool_calls: inf.calls.clone() });
        if inf.calls.is_empty() {
            return Ok(StepOutcome::Done); // 模型不再要工具 → 本轮结束
        }
        if inf.calls.len() > MAX_TOOL_CALLS_PER_STEP {
            // Assistant 已带着全部 tool_calls 入历史 —— 必须给**每一个**补占位
            // tool 结果（护栏 #4），否则下一轮 chat-completions 请求非法。
            self.push_cancelled_results(
                &inf.calls,
                0,
                "单步工具调用过多，本批全部未执行。请减少并行调用后重试。",
            );
            let msg = EventMsg::Error {
                message: format!("单步工具调用过多（{} > {}）", inf.calls.len(), MAX_TOOL_CALLS_PER_STEP),
            };
            self.emit_and_log(&msg)?;
            self.finish_turn_if_idle()?;
            return Ok(StepOutcome::Done);
        }
        match self.execute_from(&inf.calls, 0)? {
            ExecOutcome::Done => Ok(StepOutcome::More), // 工具欠一次请求 → 下一步
            ExecOutcome::Suspended => Ok(StepOutcome::Suspended),
            ExecOutcome::Aborted => {
                // 熔断/中止：立刻收轮，不再驱动下一步
                self.finish_turn_if_idle()?;
                Ok(StepOutcome::Done)
            }
        }
    }

    /// 没有挂起时结束本轮（发 TurnComplete）。
    ///
    /// 若刚结束的是**目标子任务轮**：在这里推进编排器（阶段前进/重试/判停）
    /// 并落日志快照 —— 用轮次结束这个单一事件驱动编排，而不是散在各处。
    fn finish_turn_if_idle(&mut self) -> Result<(), KernelError> {
        if matches!(self.state, KernelState::Idle) {
            let usage = (self.usage_in, self.usage_out);
            // 护栏 #9：轮末把本轮用量并入会话累计（跨轮预算的唯一记账点）
            self.session_tokens_used = self
                .session_tokens_used
                .saturating_add(usage.0.saturating_add(usage.1));
            // 并入后再看是否接近阈值：提醒必须基于**含本轮**的累计
            self.maybe_warn_session_budget()?;
            let done = EventMsg::TurnComplete {
                input_tokens: usage.0,
                output_tokens: usage.1,
            };
            self.emit_and_log(&done)?;
            self.turn_active = false;
            if self.goal_turn_in_flight {
                self.goal_turn_in_flight = false;
                let failed = self.goal_turn_failed;
                self.goal_turn_failed = false;
                let review_text = self.goal_turn_last_text.take().unwrap_or_default();
                let events =
                    self.with_goal(|g| g.on_turn_complete(usage, failed, &review_text))?;
                for ev in &events {
                    self.emit_and_log(ev)?;
                }
            }
        }
        Ok(())
    }

    /// 为未执行的 tool_call 补占位结果（护栏 #4）：事件 + 模型可见历史各一条。
    ///
    /// 从 `from` 起（含）全部补上。原因必须写进 stderr —— 空字符串会让模型
    /// 以为工具成功返回了空输出，从而原样重试。
    fn push_cancelled_results(&mut self, calls: &[ToolInvocation], from: usize, reason: &str) {
        for call in calls.iter().skip(from) {
            let ev = EventMsg::ToolCallEnd {
                id: call.id.clone(),
                exit_code: -1,
                stdout: String::new(),
                stderr: reason.to_string(),
                truncated: false,
            };
            self.emit_and_log(&ev).ok();
            self.messages.push(Message::ToolResult {
                id: call.id.clone(),
                name: call.name.clone(),
                output: cancelled_output(reason),
            });
        }
    }

    /// 护栏记账与就地提醒（#1 重复调用 / #2 预算）。
    ///
    /// 提醒**拼进该次工具结果的 stderr**：模型下一轮请求必然带上它
    /// （tool 消息是历史的一部分），且不会另造一条与工具无关的 System 消息。
    fn guard_after_call(&mut self, call: &ToolInvocation, output: &mut ToolOutput) {
        self.calls_this_turn += 1;

        // #1 连续相同调用：签名 = 工具名 + 参数稳定序列化（与 #6 doom-loop 同源）
        let sig = call_signature(call);
        if self.last_call_sig.as_deref() == Some(sig.as_str()) {
            self.repeat_count += 1;
        } else {
            self.last_call_sig = Some(sig);
            self.repeat_count = 1;
        }
        if self.repeat_count == REPEATED_CALL_WARN_AT
            && self.guard_warnings_this_turn < MAX_GUARD_WARNINGS_PER_TURN
        {
            self.guard_warnings_this_turn += 1;
            push_guard_note(
                output,
                &format!(
                    "你已连续 {REPEATED_CALL_WARN_AT} 次发起完全相同的工具调用。\
请：(1) 用已有结果推进任务；(2) 若卡住，说明阻塞点；(3) 必要时向用户求助。不要再原样重复。"
                ),
            );
        }

        // #2 单轮预算提醒（低于熔断，留一档自我纠正；只提醒一次）
        if self.calls_this_turn == TOOL_CALL_BUDGET_WARN_PER_TURN
            && self.guard_warnings_this_turn < MAX_GUARD_WARNINGS_PER_TURN
        {
            self.guard_warnings_this_turn += 1;
            push_guard_note(
                output,
                &format!(
                    "本轮已执行 {TOOL_CALL_BUDGET_WARN_PER_TURN} 次工具调用，接近上限（{TOOL_CALL_FUSE_PER_TURN}）。\
请尽快用已有结果收尾、说明结论或向用户求助。"
                ),
            );
        }
    }

    /// 执行一步产生的工具调用（从 `start` 起；审批恢复后从断点继续）。
    fn execute_from(&mut self, calls: &[ToolInvocation], start: usize) -> Result<ExecOutcome, KernelError> {
        // 护栏 #2 熔断：本轮调用数到顶就停，不再接受新调用
        if self.calls_this_turn >= TOOL_CALL_FUSE_PER_TURN {
            self.push_cancelled_results(
                calls,
                start,
                "本轮工具调用已达上限，本调用未执行。请基于已有结果收尾。",
            );
            let msg = EventMsg::Error {
                message: format!(
                    "本轮工具调用已达上限（{TOOL_CALL_FUSE_PER_TURN} 次），已停止以免失控。\
已完成的工作都在上面；继续的话再发一条消息接着做。"
                ),
            };
            self.emit_and_log(&msg)?;
            return Ok(ExecOutcome::Aborted);
        }

        // 护栏 #8：只读且已放行的连续调用可并行（上限 MAX_TOOL_CONCURRENCY）；
        // 写 / 审批 / 拒绝一律串行。并行只加速**执行**，事件仍按原序回写（T2 确定性）。
        let mut index = start;
        while index < calls.len() {
            let mut streak: Vec<usize> = Vec::new();
            let mut i = index;
            while i < calls.len() {
                if self.is_parallel_readonly(&calls[i]) {
                    streak.push(i);
                    i += 1;
                } else {
                    break;
                }
            }
            if streak.len() >= 2 {
                for chunk in streak.chunks(MAX_TOOL_CONCURRENCY) {
                    self.execute_readonly_parallel(chunk, calls)?;
                }
                index = i;
                continue;
            }
            // 不构成并行段：单条只读或任何非只读 → 串行处理 index
            self.execute_one_serial(&calls, index)?;
            if matches!(
                self.state,
                KernelState::AwaitingApproval { .. } | KernelState::AwaitingUserInput { .. }
            ) {
                return Ok(ExecOutcome::Suspended);
            }
            index += 1;
        }
        Ok(ExecOutcome::Done)
    }

    /// 本调用是否可进只读并行段：语义类别 Read，且闸门最终为 Allow（非 Ask/Deny）。
    /// **即将触发 doom-loop 闸门的调用不进并行段**（必须走串行 Ask）。
    fn is_parallel_readonly(&self, call: &ToolInvocation) -> bool {
        if self.would_trigger_loop_gate(call) {
            return false;
        }
        let kind = self.classify(call);
        if kind != CallKind::Read {
            return false;
        }
        let decision = gate(kind, self.resolution());
        let decision = match decision {
            GateDecision::Ask { .. } if self.granted.contains(&kind) => GateDecision::Allow,
            other => other,
        };
        matches!(decision, GateDecision::Allow)
    }

    /// 下一次执行该调用是否会触发 doom-loop 闸门（护栏 #6）。
    /// 判据与 #1 同源：签名与上次相同，且累计次数将达到 `DOOM_LOOP_GATE_AT`。
    fn would_trigger_loop_gate(&self, call: &ToolInvocation) -> bool {
        let sig = call_signature(call);
        self.last_call_sig.as_deref() == Some(sig.as_str())
            && self.repeat_count + 1 >= DOOM_LOOP_GATE_AT
    }

    /// 串行处理单个调用（Deny / Ask / 写 Allow / 单条只读）。
    /// 从 `execute_from` 原 for 循环体抽出来，避免并行/串行两份 match 漂移。
    fn execute_one_serial(
        &mut self,
        calls: &[ToolInvocation],
        index: usize,
    ) -> Result<(), KernelError> {
        let call = calls[index].clone();
        let kind = self.classify(&call);
        // 交互类工具（request_user_input）不经审批闸门、也不进 execute：
        // 直接挂「问用户」反向请求（Codex item/tool/requestUserInput 同形）。
        // 放在 gate 之前：问人本身就是交互，不是写/读权限问题。
        if kind == CallKind::Interactive {
            let prompt = call
                .arguments
                .get("prompt")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("需要你的输入")
                .to_string();
            let id = format!(
                "user-input-{}-{}-{}",
                self.turn_counter, self.step_counter, index
            );
            let ev = EventMsg::UserInputRequest { id: id.clone(), prompt };
            self.emit_and_log(&ev)?;
            self.pending = Some(PendingApproval {
                calls: calls.to_vec(),
                index,
                loop_gate: false,
                user_input: true,
            });
            self.state = KernelState::AwaitingUserInput { id };
            return Ok(());
        }
        // 护栏 #6：doom-loop 闸门**永远不参与自动批准** ——
        // 必须在 gate()/granted 之前拦截：FullAccess 的 Never 策略
        // 与「总是允许」类别记忆都不能把它变成 Allow。
        let is_loop = self.would_trigger_loop_gate(&call);
        let decision = if is_loop {
            GateDecision::Ask {
                detail: format!(
                    "循环检测：已连续 {} 次相同调用（阈值 {}），该闸门不参与自动批准",
                    self.repeat_count + 1,
                    DOOM_LOOP_GATE_AT
                ),
            }
        } else {
            let decision = gate(kind, self.resolution());
            match decision {
                GateDecision::Ask { .. } if self.granted.contains(&kind) => GateDecision::Allow,
                other => other,
            }
        };
        match decision {
            GateDecision::Allow => self.execute_one(&call),
            GateDecision::Deny { reason } => {
                let ev = EventMsg::ToolCallEnd {
                    id: call.id.clone(),
                    exit_code: -1,
                    stdout: String::new(),
                    stderr: String::new(),
                    truncated: false,
                };
                self.emit_and_log(&ev)?;
                self.messages.push(Message::ToolResult {
                    id: call.id,
                    name: call.name,
                    output: denied_output(&reason),
                });
                Ok(())
            }
            GateDecision::Ask { detail } => {
                let detail =
                    format!("{detail}:{} {}", call.name, call_arg_summary(&call.arguments));
                if let Some(tool) = self.tools.get(&call.name) {
                    if let Some((path, diff)) = tool.preview(&call.arguments, &self.cwd) {
                        let ev = EventMsg::PatchProposed { path, diff };
                        self.emit_and_log(&ev)?;
                    }
                }
                let id = format!(
                    "approval-{}-{}-{}",
                    self.turn_counter, self.step_counter, index
                );
                // 循环闸门的 kind 固定为 "loop" —— 宿主可与 read/write 卡片区分，
                // 且「总是允许」范围不会被错误映射到类别 granted。
                let kind_name = if is_loop { "loop" } else { call_kind_name(kind) };
                let ev = EventMsg::ApprovalRequest {
                    id: id.clone(),
                    detail,
                    kind: kind_name.to_string(),
                };
                self.emit_and_log(&ev)?;
                self.pending = Some(PendingApproval {
                    calls: calls.to_vec(),
                    index,
                    loop_gate: is_loop,
                    user_input: false,
                });
                self.state = KernelState::AwaitingApproval { id };
                Ok(())
            }
        }
    }

    /// 并行执行一段只读放行调用（每段 ≤ `MAX_TOOL_CONCURRENCY`），**按原序**回写。
    ///
    /// 执行可以乱序完成，但事件与 tool 结果必须按 `chunk` 里的下标顺序落地 ——
    /// 否则同一 Op 序列会得到不同 Event 序列，破坏 T2 回放确定性。
    fn execute_readonly_parallel(
        &mut self,
        chunk: &[usize],
        calls: &[ToolInvocation],
    ) -> Result<(), KernelError> {
        // 1. 执行前取 preview（&self；执行后文件已变，取不到）
        let previews: Vec<Option<(String, String)>> = chunk
            .iter()
            .map(|&idx| {
                self.tools
                    .get(&calls[idx].name)
                    .and_then(|t| t.preview(&calls[idx].arguments, &self.cwd))
            })
            .collect();

        // 2. 无 &mut 跑工具（只搬 Sync 件；Kernel 整体非 Sync，不能整借）
        let outputs: Vec<ToolOutput> = {
            let sandbox = self.sandbox.as_ref();
            let cwd = self.cwd.as_path();
            let mode = self.resolution().sandbox;
            let max = self.max_output_bytes;
            let tools = &self.tools;
            std::thread::scope(|scope| {
                let handles: Vec<_> = chunk
                    .iter()
                    .map(|&idx| {
                        let call = &calls[idx];
                        let tool = tools.get(&call.name);
                        scope.spawn(move || run_tool_standalone(
                            tool.as_deref(),
                            call,
                            sandbox,
                            mode,
                            cwd,
                            max,
                        ))
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().expect("只读并行线程不得 panic"))
                    .collect()
            })
        };

        // 3. 按原序回写状态与事件
        for (k, &idx) in chunk.iter().enumerate() {
            self.apply_tool_result(&calls[idx], previews[k].clone(), outputs[k].clone())?;
        }
        Ok(())
    }

    /// 只跑工具、不碰内核状态（护栏 #8 并行执行腿）。
    fn run_tool(&self, call: &ToolInvocation) -> ToolOutput {
        match self.tools.get(&call.name) {
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
        }
    }

    /// 把一次工具结果写回内核状态（护栏记账、事件、历史）。
    /// `change_before` 必须是**执行前**取到的 preview。
    fn apply_tool_result(
        &mut self,
        call: &ToolInvocation,
        change_before: Option<(String, String)>,
        mut output: ToolOutput,
    ) -> Result<(), KernelError> {
        if self.goal_turn_in_flight {
            let kind = self.classify(call);
            self.goal_turn_call_kinds.insert(call.id.clone(), kind);
        }
        // 护栏 #1/#2：提醒拼进本次结果 stderr
        self.guard_after_call(call, &mut output);
        if let Some(tool) = self.tools.get(&call.name) {
            for ev in tool.report(&call.arguments) {
                self.emit_and_log(&ev)?;
            }
        }
        if output.exit_code == 0 {
            if let Some((path, diff)) = change_before {
                let additions =
                    diff.lines().filter(|l| l.starts_with('+') && !l.starts_with("+++")).count();
                let deletions =
                    diff.lines().filter(|l| l.starts_with('-') && !l.starts_with("---")).count();
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
            stdout: output.stdout.clone(),
            stderr: output.stderr.clone(),
            truncated: output.truncated,
        };
        self.emit_and_log(&ev)?;
        // 图片附件：先落元数据事件（小、可回放），再把像素放进内存消息。
        if output.exit_code == 0 {
            if let Some(tool) = self.tools.get(&call.name) {
                if let Some(img) = tool.image_result(&call.arguments, &output) {
                    let meta = EventMsg::ImageAttached {
                        id: call.id.clone(),
                        path: img.path.clone(),
                        mime: img.mime.clone(),
                        bytes: img.bytes,
                    };
                    self.emit_and_log(&meta)?;
                    self.messages.push(Message::UserImage {
                        text: format!("[已附加图片 {}]", img.path),
                        image: img,
                    });
                }
            }
        }
        self.messages.push(Message::ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            output,
        });
        Ok(())
    }

    /// 恢复后继续同一步剩余调用；跑完则进入下一步。
    /// 落实一次审批：执行/记类别/记拒绝，然后按 `drive` 决定是否继续跑完整轮。
    ///
    /// `reason` 仅在拒绝时使用：用户的拒绝理由进入模型可见的工具结果 ——
    /// 模型因此知道"为什么被拒"，可以换个做法而不是重试同一件事
    /// （真机实测：不给理由时模型会说"工具返回空、无报错"然后原样重试）。
    ///
    /// `drive = false`（逐帧宿主）时只执行**本步剩余调用**就返回 ——
    /// 后续步骤由宿主 `Op::Pump` 逐步推进。这样审批之后也不会出现
    /// "一次调用里跑完多次网络往返"的冻结。
    fn resolve_approval(
        &mut self,
        id: ApprovalId,
        decision: Decision,
        reason: Option<String>,
        drive: bool,
    ) -> Result<(), KernelError> {
        let Some(pending) = self.pending.take() else {
            return Err(KernelError::NoPendingApproval(id));
        };
        if pending.user_input {
            // 挂起的是提问，不是审批 —— 方法用错了要响亮失败
            self.pending = Some(pending);
            return Err(KernelError::NoPendingApproval(id));
        }
        let call = pending.calls[pending.index].clone();
        self.state = KernelState::Idle;

        match decision {
            Decision::Allow => self.execute_one(&call)?,
            Decision::AllowAlways => {
                // 护栏 #6：循环闸门的「总是允许」**只放行这一次**，
                // 不得写入 granted —— 否则等于永久关掉循环检测。
                if pending.loop_gate {
                    self.execute_one(&call)?;
                } else {
                    // 记住**类别**：之后同类调用不再问。只记这一条命令的话
                    // 用户下次仍会被问 —— 那就等于没实现。
                    let kind = self.classify(&call);
                    self.granted.insert(kind);
                    self.execute_one(&call)?;
                }
            }
            Decision::Deny => {
                // 拒绝理由进 stderr（工具结果的组成部分，模型可见即已落日志）
                let deny_text = match reason.as_deref().map(str::trim) {
                    Some(r) if !r.is_empty() => format!("用户拒绝了该调用：{r}"),
                    _ => "用户拒绝了该调用".to_string(),
                };
                let ev = EventMsg::ToolCallEnd {
                    id: call.id.clone(),
                    exit_code: -1,
                    stdout: String::new(),
                    stderr: String::new(),
                    truncated: false,
                };
                self.emit_and_log(&ev)?;
                self.messages.push(Message::ToolResult {
                    id: call.id,
                    name: call.name,
                    output: denied_output(&deny_text),
                });
                // 护栏 #7：用户点「拒绝」= 停止这条路径，**不是**换下一个调用继续跑。
                // 同批剩余调用补占位（护栏 #4），本轮不再驱动模型。
                self.push_cancelled_results(
                    &pending.calls,
                    pending.index + 1,
                    "用户拒绝了前序调用，本批剩余调用未执行",
                );
                // 拒绝即停：立刻收轮（TurnComplete），不给模型再绕一圈的机会。
                return self.finish_turn_if_idle();
            }
        }
        if drive {
            // 继续同一步的剩余调用，然后进入下一步（一路跑到本轮结束）
            self.finish_step_from(&pending.calls, pending.index + 1)?;
        } else {
            // 只把本步剩余调用执行完（有界：都是工具执行，不再有模型往返）
            if pending.index + 1 < pending.calls.len() {
                match self.execute_from(&pending.calls, pending.index + 1)? {
                    ExecOutcome::Done => {}
                    ExecOutcome::Suspended => return Ok(()),
                    ExecOutcome::Aborted => return self.finish_turn_if_idle(),
                }
            }
        }
        Ok(())
    }

    /// 应答一次用户输入挂起：答复作为该工具调用的 stdout 写入模型可见历史。
    ///
    /// 与审批同构：id 单一事实源来自内核；答复原样进 `ToolCallEnd.stdout`，
    /// 模型因此知道用户回了什么（「模型可见即已落盘」）。
    fn resolve_user_input(&mut self, id: ApprovalId, response: String) -> Result<(), KernelError> {
        let Some(pending) = self.pending.take() else {
            return Err(KernelError::NoPendingApproval(id));
        };
        if !pending.user_input {
            // 挂起的是审批不是提问 —— 方法用错了要响亮失败
            self.pending = Some(pending);
            return Err(KernelError::NoPendingApproval(id));
        }
        let call = pending.calls[pending.index].clone();
        self.state = KernelState::Idle;
        let text = response.trim().to_string();
        let ev = EventMsg::ToolCallEnd {
            id: call.id.clone(),
            exit_code: 0,
            stdout: text.clone(),
            stderr: String::new(),
            truncated: false,
        };
        self.emit_and_log(&ev)?;
        self.messages.push(Message::ToolResult {
            id: call.id.clone(),
            name: call.name.clone(),
            output: ToolOutput {
                exit_code: 0,
                stdout: text,
                stderr: String::new(),
                truncated: false,
            },
        });
        // 同步：同一步剩余调用 + 驱动后续步骤（与 Approve 的 drive=true 一致）
        self.finish_step_from(&pending.calls, pending.index + 1)
    }

    fn finish_step_from(&mut self, calls: &[ToolInvocation], start: usize) -> Result<(), KernelError> {
        if start < calls.len() {
            match self.execute_from(calls, start)? {
                ExecOutcome::Done => {}
                ExecOutcome::Suspended => return Ok(()),
                ExecOutcome::Aborted => return self.finish_turn_if_idle(),
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
        self.turn_active = false;
        // 从现在起的轮次号独立（每个会话自己的轮次序列）
        self.turn_counter = 0;
        self.step_counter = 0;
        self.shell_counter = 0;
        self.steps_this_turn = 0;
        let logs = self.persistence.load().unwrap_or_default();
        self.rebuild_from_log(&logs)
    }

    /// 注入压缩策略（链式，构造后立即调用；不注入则 `/compact` 如实报未配置）。
    pub fn with_compactor(mut self, c: Box<dyn Compactor>) -> Self {
        self.compactor = c;
        self
    }

    /// 注入目标编排器。缺省 `None`：Goal 系列 Op 会如实报错
    /// （"未配置编排策略"），而不是静默假装成功。
    pub fn with_goal_orchestrator(mut self, g: Box<dyn GoalOrchestrator>) -> Self {
        self.goal = Some(g);
        self
    }

    /// 当前目标的单行状态（宿主展示用）。无活动目标为 `None`。
    pub fn goal_status(&self) -> Option<String> {
        self.goal.as_ref().and_then(|g| g.goal_id()).map(|id| format!("目标 {id} 进行中"))
    }

    /// 当前目标完整快照（app-server `thread/goal/get`）。无活动目标为 `None`。
    pub fn goal_snapshot(&self) -> Option<neo_protocol::GoalSnapshot> {
        self.goal.as_ref().and_then(|g| g.snapshot())
    }

    /// 进行中的轮次 id（`turn-N`）。无活动轮为 `None`。Codex `turn/steer` 前置条件。
    pub fn active_turn_id(&self) -> Option<String> {
        if self.turn_active {
            Some(format!("turn-{}", self.turn_counter))
        } else {
            None
        }
    }

    /// 技能列表（Codex `skills/list`）：`(name, description)`。
    pub fn skill_list(&self) -> Vec<(String, String)> {
        self.skills
            .names()
            .into_iter()
            .map(|n| {
                let d = self
                    .skills
                    .get(&n)
                    .map(|s| s.description.clone())
                    .unwrap_or_default();
                (n.to_string(), d)
            })
            .collect()
    }

    /// 沙箱档位（fs/* 等装配层判定用）。
    pub fn sandbox_mode(&self) -> neo_protocol::SandboxMode {
        self.resolution().sandbox
    }

    /// 沙箱契据（写文件必须经此 —— 不给宿主旁路）。
    pub fn sandbox(&self) -> &dyn SandboxBackend {
        self.sandbox.as_ref()
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
                        // 目标编排状态从快照事件重建：最后一条快照即当前状态
                        //（kill 进程后重启能续跑的核心）。没有编排器时跳过
                        //（状态无处安放，日志本身仍完整可审计）。
                        Ok(ev) => {
                            if let Some(g) = self.goal.as_mut() {
                                g.observe(&ev);
                            }
                            self.replay_event(ev, &mut rebuilt, &mut pending_calls);
                        }
                        // 坏事件跳过（schema 门禁挡在前面，这里兜底不 panic）
                        Err(_) => {}
                    }
                }
                _ => {}
            }
        }
        let n = rebuilt.len();
        ensure_tool_call_pairing(&mut rebuilt);
        self.messages = rebuilt;
        n
    }

    /// 回放单条事件对**消息历史**的作用（rebuild_from_log 的 match 体）。
    ///
    /// 抽成方法是为了让"喂编排器 observe"与"重建消息历史"在同一条
    /// 事件流上各取所需 —— 两件事都做，但互不掺和。
    fn replay_event(
        &mut self,
        ev: EventMsg,
        rebuilt: &mut Vec<Message>,
        pending_calls: &mut Vec<ToolInvocation>,
    ) {
        match ev {
            // 指令是系统提示词的组成，回放必须还原**当时**那一份，
            // 而不是重新读盘（AGENTS.md 可能已被改）。还原后重拼提示词，
            // 否则回放出的请求与真实请求系统提示词不一致。
            EventMsg::InstructionsLoaded { sources, block, truncated } => {
                self.instructions = crate::instructions::Instructions {
                    sources,
                    block,
                    truncated,
                };
                self.instructions_logged = true;
                self.refresh_system_prompt();
            }
            EventMsg::UserSubmitted { text } => {
                rebuilt.push(Message::User(text));
            }
            // 注入块拼到刚推入的用户消息之后：submit 里就是
            // `{text}\n\n{block}`，这里必须还原同一顺序。
            EventMsg::RefsResolved { block, .. } => {
                if !block.is_empty() {
                    if let Some(Message::User(t)) = rebuilt.last_mut() {
                        t.push_str("\n\n");
                        t.push_str(&block);
                    }
                }
            }
            EventMsg::ToolCallBegin { id, name, arguments } => {
                pending_calls.push(ToolInvocation { id, name, arguments });
            }
            EventMsg::AgentMessageDone { text } => {
                rebuilt.push(Message::Assistant {
                    text,
                    tool_calls: std::mem::take(pending_calls),
                });
            }
            EventMsg::ContextCompacted { removed_messages, summary } => {
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
            EventMsg::ToolCallEnd { id, exit_code, stdout, stderr, truncated } => {
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
            // 图片：日志只存路径元数据。仍在盘上则重读进内存；文件已变/已删
            // 则降级为纯文字（诚实边界：不假装还能看到当时的像素）。
            EventMsg::ImageAttached { path, mime, bytes, .. } => {
                let image = std::fs::read(&path).ok().filter(|raw| !raw.is_empty()).map(|raw| {
                    neo_protocol::ImageAttachment {
                        mime: mime.clone(),
                        data_base64: base64_encode(&raw),
                        path: path.clone(),
                        detail: None,
                        bytes: raw.len() as u64,
                    }
                });
                match image {
                    Some(image) => rebuilt.push(Message::UserImage {
                        text: format!("[已附加图片 {path}]"),
                        image,
                    }),
                    None => rebuilt.push(Message::User(format!(
                        "[图片不可用：{path}（{mime}，{bytes}B）]"
                    ))),
                }
            }
            _ => {}
        }
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
        p: std::sync::Arc<dyn ModelProvider>,
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

    /// 设置 / 清除会话 token 预算。`0` = 清除（不限）。
    pub fn set_session_token_budget(&mut self, budget: u64) {
        self.session_token_budget = if budget == 0 { None } else { Some(budget) };
        self.session_budget_warned = false;
    }

    /// 当前会话预算（`None` = 未设限）与已用量。
    pub fn session_token_budget(&self) -> (Option<u64>, u64) {
        (self.session_token_budget, self.session_tokens_used)
    }

    fn session_budget_exceeded(&self) -> bool {
        match self.session_token_budget {
            Some(limit) => self.session_tokens_used >= limit,
            None => false,
        }
    }

    fn check_session_token_budget(&self) -> Result<(), KernelError> {
        if self.session_budget_exceeded() {
            return Err(KernelError::SessionTokenBudgetExceeded {
                used: self.session_tokens_used,
                limit: self.session_token_budget.unwrap_or(0),
            });
        }
        Ok(())
    }

    /// 接近预算时注入一次提醒（护栏 #9）：模型侧 System + 用户侧 Error 事件。
    ///
    /// 只提醒一次（`session_budget_warned`）；设新预算时重置。
    fn maybe_warn_session_budget(&mut self) -> Result<(), KernelError> {
        let Some(limit) = self.session_token_budget else { return Ok(()) };
        if self.session_budget_warned {
            return Ok(());
        }
        let threshold = limit.saturating_mul(SESSION_BUDGET_WARN_PERCENT) / 100;
        if self.session_tokens_used < threshold {
            return Ok(());
        }
        self.session_budget_warned = true;
        let msg = format!(
            "[guard] 会话 token 预算已用 {}/{}（≥{}%）。请尽快收尾并给出已完成部分的小结；\
超限后将优雅停止，提高预算或开新会话才能继续。",
            self.session_tokens_used, limit, SESSION_BUDGET_WARN_PERCENT
        );
        // 模型可见（进历史）+ 用户可见（事件）
        self.messages.push(Message::System(msg.clone()));
        self.emit_and_log(&EventMsg::Error { message: msg })?;
        Ok(())
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
            .and_then(|t| t.preview(&call.arguments, &self.cwd));
        let output = self.run_tool(call);
        self.apply_tool_result(call, change_before, output)
    }

    fn emit_and_log(&mut self, ev: &EventMsg) -> Result<(), KernelError> {
        // 目标子任务轮的失败判据（审查代理）：轮内出现 Error 事件，或
        // 工具以非零退出码结束 —— 单点记账，新的出错路径自动被算进去。
        // 审批拒绝的调用也以非零结束，同样计入（子任务被拦 = 没完成）。
        if self.goal_turn_in_flight {
            match ev {
                EventMsg::Error { .. } => self.goal_turn_failed = true,
                EventMsg::ToolCallEnd { id, exit_code, .. } if *exit_code != 0 => {
                    // 只读调用失败 = 探测失败（模型核验时 cat 一个不存在的
                    // 路径很正常），不判死整轮审查；写入/网络失败才算。
                    // 真机回归（智谱）里一次无害的探测失败曾多花一轮
                    // Code→Review（约 40% 预算）—— 就是这条边界要修的。
                    let is_read_probe = self
                        .goal_turn_call_kinds
                        .get(id)
                        .map(|k| *k == CallKind::Read)
                        .unwrap_or(false);
                    if !is_read_probe {
                        self.goal_turn_failed = true;
                    }
                }
                // 多步轮会有多条 AgentMessageDone：最后一条 = 模型的最终结论
                EventMsg::AgentMessageDone { text } => {
                    self.goal_turn_last_text = Some(text.clone());
                }
                _ => {}
            }
        }
        self.outbox.push(ev.clone());
        self.log("event", ev)
    }

    /// 在活动编排器上执行操作；未配置编排策略时如实报错。
    fn with_goal<F>(&mut self, f: F) -> Result<Vec<EventMsg>, KernelError>
    where
        F: FnOnce(&mut Box<dyn GoalOrchestrator>) -> Vec<EventMsg>,
    {
        match self.goal.as_mut() {
            Some(g) => Ok(f(g)),
            None => Err(KernelError::GoalUnavailable("未配置编排策略".into())),
        }
    }

    /// GoalPause/Resume 带目标 id：与当前目标不一致是**参数错误**
    /// （用户想暂停的可能是另一个目标），拒绝而不是悄悄作用于错的那个。
    fn check_goal_id(&self, want: &str) -> Result<(), KernelError> {
        match self.goal.as_ref().and_then(|g| g.goal_id()) {
            Some(id) if id == want => Ok(()),
            Some(id) => Err(KernelError::GoalUnavailable(format!(
                "目标 id 不匹配：当前 {id}，请求 {want}"
            ))),
            None => Err(KernelError::GoalUnavailable("没有活动目标".into())),
        }
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

enum ExecOutcome {
    Done,
    Suspended,
    /// 用户已拒绝 / 预算熔断：本批剩余调用不再执行，本轮也到此为止
    /// （护栏 #7 `continue_loop_on_deny` 默认 false —— 拒绝按钮必须真能停）。
    Aborted,
}

/// 单步推进的结果。与 `ExecOutcome` 分开：一个是"这一步的工具执行完了吗"，
/// 一个是"整轮推进到哪了"。
enum StepOutcome { More, Suspended, Done }

fn denied_output(reason: &str) -> ToolOutput {
    ToolOutput { exit_code: -1, stdout: String::new(), stderr: reason.into(), truncated: false }
}

/// 标准 base64 编码（无换行）。L2 不引 crate：自实现 ~25 行，调试链浅。
fn base64_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { T[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}

/// 未执行的 tool_call 占位结果（护栏 #4）。
///
/// **内容必须非空且写清原因**：chat-completions 方言要求 assistant 的每个
/// `tool_call` 后都有对应 `role:"tool"` 消息；空占位会让模型以为工具返回了空结果。
fn cancelled_output(reason: &str) -> ToolOutput {
    denied_output(reason)
}

/// 在独立线程里跑一个工具（护栏 #8 并行执行腿）。
///
/// **不碰 `Kernel`** —— 它非 `Sync`（内含在飞流）。只传 Sync 件：
/// `Arc` 工具、沙箱引用、模式、cwd、上限。
fn run_tool_standalone(
    tool: Option<&dyn Tool>,
    call: &ToolInvocation,
    sandbox: &dyn SandboxBackend,
    mode: SandboxMode,
    cwd: &std::path::Path,
    max_output_bytes: usize,
) -> ToolOutput {
    match tool {
        Some(tool) => {
            let ctx = ToolCtx { sandbox, mode, cwd, max_output_bytes };
            tool.execute(&call.arguments, &ctx)
        }
        None => ToolOutput {
            exit_code: -1,
            stdout: String::new(),
            stderr: format!("未知工具：{}", call.name),
            truncated: false,
        },
    }
}

/// 把护栏提醒拼进工具结果的 stderr（模型可见的那一路）。
fn push_guard_note(output: &mut ToolOutput, note: &str) {
    if !output.stderr.is_empty() {
        output.stderr.push('\n');
    }
    output.stderr.push_str("[guard] ");
    output.stderr.push_str(note);
}

/// 为历史上悬空的 tool_call 补占位 tool 结果（护栏 #4 的回放侧）。
///
/// 调用方已重建完消息；这里只补「assistant 声明了 tool_call、却没有任何
/// tool 结果」的缺口。不补的话，切换会话后的下一轮请求对真实 provider 非法
/// （OpenAI 要求 tool_call 与 tool 消息成对）。
fn ensure_tool_call_pairing(rebuilt: &mut Vec<Message>) {
    let answered: std::collections::BTreeSet<String> = rebuilt
        .iter()
        .filter_map(|m| match m {
            Message::ToolResult { id, .. } => Some(id.clone()),
            _ => None,
        })
        .collect();
    let mut missing: Vec<ToolInvocation> = Vec::new();
    for m in rebuilt.iter() {
        if let Message::Assistant { tool_calls, .. } = m {
            for c in tool_calls {
                if !answered.contains(&c.id) {
                    missing.push(c.clone());
                }
            }
        }
    }
    for c in missing {
        rebuilt.push(Message::ToolResult {
            id: c.id,
            name: c.name,
            output: cancelled_output("[cancelled] 未执行：中断或超限，历史中无工具结果"),
        });
    }
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
