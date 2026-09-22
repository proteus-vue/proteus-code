//! L5 HOST · app-server —— stdio 上的 JSON-RPC 2.0
//!
//! # 为什么需要它
//!
//! 内核此前只有一种跨进程接口：Web 宿主的回环 HTTP + SSE（6 条路由，
//! 只映射 4 个 `Op`）。而**已有自己进程**的客户端（编辑器、IDE、脚本）
//! 需要的是一条通用线协议：全量 `Op` 可发、全量事件可收、审批可应答、
//! 握手能协商版本。本宿主就是这条线协议 —— 形态对齐 Codex 的 `app-server`，
//! 语义仍全部来自 `neo-protocol`（本 crate 不定义任何新语义）。
//!
//! ```text
//!  客户端进程 ──stdin（请求行）──▶ 本宿主 ──mpsc(Op)──▶ 内核线程
//!  客户端进程 ◀─stdout（响应 + 事件行）── 本宿主 ◀─事件批─┘
//! ```
//!
//! # 铁律（与 Web 宿主同一条）
//!
//! 不含业务逻辑：只解析 JSON-RPC、把方法转成 `Op`、把事件写成通知。
//! "事件 → 用户可见事实"的映射仍在协议层（`facts_of`），因此 T6 的
//! 宿主等价断言对本宿主同样成立（登记在 `neo-mock/tests/conformance.rs`）。
//!
//! # 与 Web 宿主的四处刻意不同
//!
//! 1. **传输是 stdio，不是回环端口**：进程的标准流就是信任边界，故不需要
//!    访问令牌；也不需要 TLS —— 客户端与内核是父子进程。
//! 2. **事件通知的载荷嵌在 `payload` 下**（`{"seq":1,"kind":"tool_call_begin",
//!    "payload":{…}}`），不摊平到顶层：摊平会撞键 —— `ApprovalRequest` 自带
//!    `kind` 字段（内核判定的调用类别 read/write/network/interactive），
//!    与通知的 `kind`（事件名）同键，必然丢一个。嵌套后两者都在，且与
//!    JSONL 会话日志的记录形状（`v/ts/seq/kind/payload`）一致。
//! 3. **审批是"通知 + 应答方法"**，不是服务端发起的 JSON-RPC 请求：
//!    审批 id 由内核单一事实源给出（`ApprovalRequest.id`），客户端用
//!    `approval/respond` 应答同一个 id。改造成服务端请求就要再引入一套
//!    请求 id 与内核 id 的映射 —— 两套身份必然漂移。
//! 4. **`turn/start` 立即返回、进度走通知**：响应语义是"已受理"。
//!
//! # 不做的（诚实边界）
//!
//! - **事件不重放**：stdio 连接的生命期就是进程生命期，客户端不存在
//!   "断线重连到同一会话"（重连 = 新进程 = 新会话）。跨进程续聊的正道是读
//!   会话日志（JSONL）重建，而不是在这条传输上加缓冲。
//! - **一个连接一个会话**：内核本身是单会话串行模型（`Kernel` 不是 `Sync`），
//!    多会话多路复用属于 L2 的课题，宿主层不伪造它。
//! - **客户端能力不参与降级**：握手时服务端单向声明 `HostCapabilities`，
//!    客户端读它决定怎么渲染；反过来（客户端能力决定内核降级）留待需要时加。

pub mod jsonrpc;
pub mod serve;

pub use jsonrpc::{Action, ClientInfo, InitializeParams, RpcError, PROTOCOL_VERSION};
pub use serve::{serve, serve_stdio};

use neo_core::{DiffSupport, HostBackend, HostCapabilities, ImageSupport};
use neo_protocol::{EventMsg, Fact};
use serde_json::{json, Value};

/// app-server 宿主的 `HostBackend` 视图（T6 宿主等价性的比较对象）。
///
/// 与 `neo_host_web::WebFacts` 同构：只累积事件、按协议层的 `facts_of`
/// 抽取事实 —— 抽取器是同一个，所以"宿主等价"是构造上成立的，不是巧合。
pub struct AppServerFacts {
    events: Vec<EventMsg>,
}

impl AppServerFacts {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }
}

impl Default for AppServerFacts {
    fn default() -> Self {
        Self::new()
    }
}

impl HostBackend for AppServerFacts {
    fn id(&self) -> &'static str {
        "app-server"
    }

    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities {
            // 诚实取值：线协议里没有图片载体，也没有富文本渲染 —— 渲染发生在
            // 客户端进程里，而那是本宿主看不见的。宁可声明"不支持"，
            // 也不要让工具以为它能发图片（那正是"让工具去猜宿主"的老路）。
            images: ImageSupport::None,
            rich_text: false,
            interactive_prompt: true,   // 审批是线协议的一等方法
            diffs: DiffSupport::Text,   // unified diff 以文本送达；hunk 级交互未上线
        }
    }

    fn consume(&mut self, event: &EventMsg) -> Result<(), String> {
        self.events.push(event.clone());
        Ok(())
    }

    fn facts(&self) -> Vec<Fact> {
        neo_protocol::facts_of(&self.events)
    }
}

/// 宿主能力 → JSON（握手响应里的 `host` 成员）。
///
/// 手写而不是给 `HostCapabilities` 加 `Serialize`：那是 L2 的类型，给它加
/// 序列化属性等于让线协议的形状去约束内核类型；映射留在宿主层，内核不变。
pub fn host_capabilities_json() -> Value {
    let caps = AppServerFacts::new().capabilities();
    json!({
        "id": "app-server",
        "capabilities": {
            "images": image_support_name(caps.images),
            "rich_text": caps.rich_text,
            "interactive_prompt": caps.interactive_prompt,
            "diffs": diff_support_name(caps.diffs),
        },
    })
}

fn image_support_name(v: ImageSupport) -> &'static str {
    match v {
        ImageSupport::None => "none",
        ImageSupport::Inline => "inline",
        ImageSupport::External => "external",
    }
}

fn diff_support_name(v: DiffSupport) -> &'static str {
    match v {
        DiffSupport::None => "none",
        DiffSupport::Text => "text",
        DiffSupport::Hunk => "hunk",
    }
}
