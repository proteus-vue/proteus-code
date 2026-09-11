//! `neo` —— multitool 入口
//!
//! ```
//! neo exec "<task>" [--mode <plan|confirm|default|auto-edit|full>] [--json] [--workspace <dir>]
//! neo serve          # Web 宿主（浏览器界面 + SSE 事件流）
//! neo                # TUI 宿主（真终端交互）
//! ```
//!
//! **main 只做参数解析与装配**，业务在 L2/L3、渲染在 L5 宿主。
//! 这是「宿主不含业务逻辑」的落地点：换宿主不改这里之外任何东西。

use neo_config::Config;
use neo_exec::{build_kernel, describe_mode, mode_short, run_task, ExecOptions};
use neo_protocol::{Decision, EventMsg};
use neo_protocol::ExecMode;
use std::path::PathBuf;
use std::sync::Arc;

const USAGE: &str = r#"neo —— 用 Rust 重构的编程 Agent 内核

用法：
  neo exec "<任务>" [选项]     无头跑一轮（真实模型 + 真实沙箱）
  neo serve [选项]             启动 Web 宿主（浏览器打开提示的地址）
  neo [选项]                   启动 TUI 宿主（需真终端）

exec 选项：
  --mode <plan|confirm|default|auto-edit|full>   执行模式（默认 default）
  --workspace <dir>            工作区（默认当前目录）
  --max-steps <n>              步数上限（默认 16）
  --allow-writes               无人值守时自动批准写操作（默认拒绝）
  --json                       输出 JSON（便于脚本消费）
  --provider <deepseek|mock|selftest|demo>   模型后端（默认 deepseek）
                                selftest = 按脚本调用一次工具，验证完整链路（无需 key）

serve 选项：
  --addr <host:port>           监听地址（默认 127.0.0.1:8787）
  --mode <...>                 执行模式（默认 default；Web 有交互审批，不需要放水）
  --workspace <dir>            工作区（默认当前目录）
  --provider <...>             模型后端（同 exec）

环境变量：
  DEEPSEEK_API_KEY             必需（provider=deepseek 时）
  DEEPSEEK_BASE_URL            可选，默认 api.deepseek.com
  DEEPSEEK_MODEL               可选，默认 deepseek-chat
"#;

fn parse_mode(s: &str) -> Result<ExecMode, String> {
    Ok(match s {
        "plan" => ExecMode::Plan,
        "confirm" => ExecMode::ConfirmBefore,
        "default" => ExecMode::Default,
        "auto-edit" => ExecMode::AutoEdit,
        "full" => ExecMode::FullAccess,
        other => return Err(format!("未知模式 {other}")),
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("exec") => cmd_exec(&args[1..]),
        Some("tui") => cmd_tui(&args[1..]),
        Some("serve") => cmd_serve(&args[1..]),
        Some("help") | Some("--help") | Some("-h") => {
            println!("{USAGE}");
            0
        }
        // 无参数，或首参是选项（如 `neo --provider mock`）→ 默认进 TUI。
        // 用法里写的是 `neo [选项]`，因此裸选项必须等价于 `neo tui [选项]`。
        None => cmd_tui(&args[..]),
        Some(flag) if flag.starts_with('-') => cmd_tui(&args[..]),
        Some(other) => {
            // 很可能是把选项写在了子命令前面，或拼错了子命令
            eprintln!("[neo] 未知子命令：{other}");
            eprintln!("      若想启动 TUI，请用：neo tui <选项>  或直接：neo <选项>");
            eprintln!("      查看全部用法：neo --help\n");
            2
        }
    };
    std::process::exit(code);
}

/// 启动 Web 宿主（浏览器界面 + SSE 事件流）。
///
/// 内核与 HTTP 分离：内核独占一个工作线程（`Kernel` 不需要 `Sync`），
/// HTTP 线程只做协议转换。审批在浏览器里交互，所以不需要 exec 的"默认拒绝"策略。
///
/// 注意事件流**不重放**：浏览器必须在提交任务前先连上 `/api/events`，
/// 否则这一轮事件会发给零个订阅者。这是有界广播的必然结果，见 PROJECT_MEMORY §4.9。
fn cmd_serve(args: &[String]) -> i32 {
    let mut workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut provider = "deepseek".to_string();
    let mut bind = "127.0.0.1:8787".to_string();
    let mut mode = ExecMode::Default;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--addr" => {
                i += 1;
                match args.get(i) {
                    Some(a) => bind = a.clone(),
                    None => {
                        eprintln!("[neo] --addr 需要一个 host:port");
                        return 2;
                    }
                }
            }
            "--workspace" => {
                i += 1;
                match args.get(i) {
                    Some(p) => workspace = PathBuf::from(p),
                    None => {
                        eprintln!("[neo] --workspace 需要一个目录");
                        return 2;
                    }
                }
            }
            "--provider" => {
                i += 1;
                match args.get(i) {
                    Some(p) => provider = p.clone(),
                    None => {
                        eprintln!("[neo] --provider 需要一个值");
                        return 2;
                    }
                }
            }
            "--mode" => {
                i += 1;
                match args.get(i).map(|s| parse_mode(s)) {
                    Some(Ok(m)) => mode = m,
                    Some(Err(e)) => {
                        eprintln!("[neo] {e}");
                        return 2;
                    }
                    None => {
                        eprintln!("[neo] --mode 需要一个值");
                        return 2;
                    }
                }
            }
            other => {
                eprintln!("[neo] serve 不认识的参数：{other}");
                return 2;
            }
        }
        i += 1;
    }

    let Some(models) = build_models(&provider) else {
        return 2;
    };
    let sandbox = Arc::new(neo_sandbox_local::LocalSandbox::new(&workspace));
    let persistence = Box::new(neo_session_local::JsonlPersistence::new(
        workspace.join(".neo/sessions/neo-web.jsonl"),
    ));
    let opts = ExecOptions { mode, max_steps: 32, ..Default::default() };
    let mut kernel = build_kernel("neo-web", &workspace, &opts, models, sandbox, persistence);

    // 内核线程的闭包：只做「Op 进 / 事件出」，不含任何 HTTP 细节。
    let (server, kernel_thread) = match neo_host_web::start(&bind, move |op| {
        kernel
            .submit(op)
            .unwrap_or_else(|e| vec![EventMsg::Error { message: e.to_string() }])
    }) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[neo] 无法监听 {bind}：{e}");
            return 2;
        }
    };

    eprintln!("[neo] 工作区 {}", workspace.display());
    eprintln!("[neo] 模式   {}", describe_mode(mode));
    eprintln!("[neo] 模型   {provider}");
    eprintln!("[neo] Web 宿主 http://{}  （Ctrl-C 退出）", server.addr);
    eprintln!("       注意：事件流不重放，浏览器页面会先自动连上 SSE 再提交任务");
    // 内核线程在 op 通道关闭前不会退出，join 即"服务于请求直到进程结束"。
    let _ = kernel_thread.join();
    0
}

fn cmd_exec(args: &[String]) -> i32 {
    let mut opts = ExecOptions::default();
    let mut workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut provider = "deepseek".to_string();
    let mut task_parts: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => opts.json = true,
            "--allow-writes" => opts.on_approval = Decision::Allow,
            "--mode" => {
                i += 1;
                match args.get(i).map(|s| parse_mode(s)) {
                    Some(Ok(m)) => opts.mode = m,
                    Some(Err(e)) => {
                        eprintln!("[neo] {e}");
                        return 2;
                    }
                    None => {
                        eprintln!("[neo] --mode 需要一个值");
                        return 2;
                    }
                }
            }
            "--workspace" => {
                i += 1;
                match args.get(i) {
                    Some(p) => workspace = PathBuf::from(p),
                    None => {
                        eprintln!("[neo] --workspace 需要一个目录");
                        return 2;
                    }
                }
            }
            "--max-steps" => {
                i += 1;
                match args.get(i).and_then(|s| s.parse().ok()) {
                    Some(n) => opts.max_steps = n,
                    None => {
                        eprintln!("[neo] --max-steps 需要正整数");
                        return 2;
                    }
                }
            }
            "--provider" => {
                i += 1;
                match args.get(i) {
                    Some(p) => provider = p.clone(),
                    None => {
                        eprintln!("[neo] --provider 需要一个值");
                        return 2;
                    }
                }
            }
            other => task_parts.push(other.to_string()),
        }
        i += 1;
    }

    if task_parts.is_empty() {
        eprintln!("[neo] 缺少任务描述。示例：neo exec \"列出当前目录的文件\"");
        return 2;
    }
    opts.task = task_parts.join(" ");

    // ── 装配：provider / sandbox / persistence ──────────────────────────
    let Some(models) = build_models(&provider) else {
        return 2;
    };

    let sandbox = Arc::new(neo_sandbox_local::LocalSandbox::new(&workspace));
    let persistence = Box::new(neo_session_local::JsonlPersistence::new(
        workspace.join(".neo/sessions/neo-cli.jsonl"),
    ));

    if !opts.json {
        eprintln!("[neo] 工作区 {}", workspace.display());
        eprintln!("[neo] 模式   {}", describe_mode(opts.mode));
        eprintln!(
            "[neo] 沙箱   {}",
            if neo_core::SandboxBackend::supports(sandbox.as_ref(), neo_config::resolve(opts.mode).sandbox) {
                "已启用（OS 级强制）"
            } else {
                "本平台无实现 → 受限档位将被拒绝（fail-closed）"
            }
        );
        eprintln!("[neo] 模型   {provider}\n");
    }

    let kernel = build_kernel(
        "neo-cli",
        &workspace,
        &opts,
        models,
        sandbox,
        persistence,
    );

    let (ok, output) = run_task(kernel, &opts);
    println!("{output}");
    if ok { 0 } else { 1 }
}

// 让 `Config` 与 `neo_core` 在本 crate 内可见（装配需要它们的类型）
#[allow(unused_imports)]
use neo_core as _neo_core;
#[allow(unused_imports)]
use Config as _Config;

/// 启动 TUI（交互式）。
///
/// TUI 需要终端能力（原始模式、光标定位），因此**必须真终端**；
/// 在管道 / CI 下会明确报错而不是把终端搞乱。
fn cmd_tui(args: &[String]) -> i32 {
    let workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    // 与 exec 一样支持 --provider 与 --mode：TUI 也必须能离线用（无 key）。
    let mut provider = "deepseek".to_string();
    let mut mode = ExecMode::Default;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--provider" => {
                i += 1;
                match args.get(i) {
                    Some(p) => provider = p.clone(),
                    None => {
                        eprintln!("[neo] --provider 需要一个值");
                        return 2;
                    }
                }
            }
            "--mode" => {
                i += 1;
                match args.get(i).map(|s| parse_mode(s)) {
                    Some(Ok(m)) => mode = m,
                    Some(Err(e)) => {
                        eprintln!("[neo] {e}");
                        return 2;
                    }
                    None => {
                        eprintln!("[neo] --mode 需要一个值");
                        return 2;
                    }
                }
            }
            other => {
                eprintln!("[neo] tui 未知参数：{other}");
                return 2;
            }
        }
        i += 1;
    }

    let opts = ExecOptions { mode, ..ExecOptions::default() };
    let models = match build_models(&provider) {
        Some(m) => m,
        None => return 2,
    };
    let sandbox = Arc::new(neo_sandbox_local::LocalSandbox::new(&workspace));
    // 会话库：所有会话都落在这个目录下（一个会话 = 一个 .jsonl）
    let sessions_dir = workspace.join(".neo/sessions");
    let store = neo_session_store::SessionStore::open(&sessions_dir);
    // 当前会话 id：优先接续最近一个（符合"打开就该继续"的直觉），
    // 没有历史时才新建。--new 可强制开新会话。
    let want_new = args.iter().any(|a| a == "--new");
    let session_id = if want_new || store.is_empty() {
        let id = store.new_id();
        eprintln!("[neo] 新建会话 {id}");
        id
    } else {
        let id = store
            .list()
            .first()
            .map(|m| m.id.clone())
            .unwrap_or_else(|| store.new_id());
        eprintln!("[neo] 继续会话 {id}（/sessions 可切换，/new 新建）");
        id
    };
    let persistence = Box::new(neo_session_local::JsonlPersistence::new(store.path_for(&session_id)));
    let kernel = build_kernel(&session_id, &workspace, &opts, models, sandbox, persistence);

    // 首屏信息由 CLI 装配（宿主不读环境）—— 与 exec 启动时打印的那三行同源，
    // 避免"命令行提示"与"TUI 首屏"两处各说一套。
    let about = neo_host_tui::About {
        version: env!("CARGO_PKG_VERSION").to_string(),
        model: provider.clone(),
        mode: describe_mode(mode),
        mode_short: mode_short(mode).to_string(),
        workspace: workspace.display().to_string(),
        branch: detect_branch(&workspace),
        // 模型清单由 CLI 注入（宿主不持有内核，也不需要知道注册表）
        models: kernel
            .available_models()
            .into_iter()
            .map(|m| (m.name, m.description, m.production))
            .collect(),
        current_model: kernel.current_model().to_string(),
        // 每次启动换一个示例（不需要真随机：只要别每次都一样）
        example: neo_host_tui::pick_example(),
        // deepseek-chat 的上下文窗口。写错不如不写：侧栏在 0 时显示"上限未知"。
        context_limit: 64_000,
        session: session_id.to_string(),
    };

    // 信任门：已信任过的工作区不重复打扰（记录在 ~/.neo/trusted.json）
    let gate = if mode == ExecMode::FullAccess {
        // 全权档位本身就是"我承担风险"的显式选择，再问一次没有增量信息
        None
    } else if neo_host_tui::trust::is_trusted(&workspace) {
        None
    } else {
        Some(workspace.clone())
    };

    // 主题偏好从用户目录读（首次用默认）；由 CLI 注入，宿主不自己读配置
    let theme_name = neo_host_tui::theme::load_preference();

    // 会话控制：宿主调契据，CLI 执行（含内核换会话与历史重建）。
    // 内核用 Rc<RefCell> 与 submit 闭包共享 —— TUI 单线程，无需锁。
    let kernel = std::rc::Rc::new(std::cell::RefCell::new(kernel));
    let mut sessions = TuiSessions::new(kernel.clone(), store);
    let submit_kernel = kernel.clone();
    // 注入 submit：TUI 只认契据，业务在 kernel
    let result = neo_host_tui::run(about, gate, theme_name, &mut sessions, move |op| {
        submit_kernel.borrow_mut().submit(op).map_err(|e| e.to_string())
    });
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("[neo] TUI 启动失败：{e}");
            2
        }
    }
}


/// TUI 的会话控制实现：持有内核与会话库，执行切换/新建/删除。
///
/// 它活在 CLI 层而不是宿主里，因为**只有 CLI 知道内核怎么装配** ——
/// 宿主只调用契据（`SessionControl`），不碰内核类型。
struct TuiSessions {
    /// 与 submit 闭包共享同一内核（TUI 是单线程，`Rc<RefCell>` 足够且无锁）
    kernel: std::rc::Rc<std::cell::RefCell<neo_core::Kernel>>,
    /// 会话库：会话的列举/新建/删除都在这里（日志路径由它给出）
    store: neo_session_store::SessionStore,
}

impl TuiSessions {
    fn new(
        kernel: std::rc::Rc<std::cell::RefCell<neo_core::Kernel>>,
        store: neo_session_store::SessionStore,
    ) -> Self {
        Self { kernel, store }
    }

    /// 为某个会话 id 造一个 JSONL 持久化（指向该会话自己的文件）。
    fn persistence_for(&self, id: &str) -> Box<dyn neo_core::SessionPersistence> {
        Box::new(neo_session_local::JsonlPersistence::new(self.store.path_for(id)))
    }
}

impl neo_host_tui::SessionControl for TuiSessions {
    fn list(&self) -> Vec<(String, String, usize)> {
        self.store
            .list()
            .into_iter()
            .map(|m| (m.id, m.title, m.records))
            .collect()
    }

    fn switch(&mut self, id: &str) -> Result<Vec<neo_protocol::EventMsg>, String> {
        if !self.store.exists(id) {
            return Err(format!("会话 {id} 不存在"));
        }
        let p = self.persistence_for(id);
        // 内核换会话并重建历史
        let mut k = self.kernel.borrow_mut();
        k.switch_session(id, p);
        // 把重建出的历史**转回事件流**给宿主重画转录。
        // 从落盘日志直接取 event 记录，保持与原始流一致。
        let logs = k.log_for_test();
        drop(k);
        let mut history = Vec::new();
        for rec in logs {
            if rec.kind == "event" {
                if let Ok(ev) =
                    serde_json::from_value::<neo_protocol::EventMsg>(rec.payload.clone())
                {
                    history.push(ev);
                }
            }
        }
        Ok(history)
    }

    fn create(&mut self) -> Result<String, String> {
        let id = self.store.new_id();
        let p = self.persistence_for(&id);
        self.kernel.borrow_mut().switch_session(id.clone(), p);
        Ok(id)
    }

    fn delete(&mut self, id: &str) -> Result<bool, String> {
        if id == self.kernel.borrow().session_id() {
            return Err("不能删除当前正在使用的会话（先切换到别的会话）".into());
        }
        self.store.delete(id).map_err(|e| e.to_string())
    }

    fn current(&self) -> String {
        self.kernel.borrow().session_id().to_string()
    }
}

/// 探测 git 分支（零依赖：读 `.git/HEAD`）。
///
/// 用文件读而不是 `git` 子进程：启动 TUI 时不该为一个装饰性信息
/// 去 fork 一个进程，也不该假设用户装了 git。
/// 探测不到就返回空串，界面按"不在仓库里"处理（不显示冒号）。
fn detect_branch(ws: &std::path::Path) -> String {
    // 工作区可能在仓库的子目录里，逐级向上找 .git
    let mut cur = Some(ws);
    while let Some(dir) = cur {
        let head = dir.join(".git").join("HEAD");
        if let Ok(raw) = std::fs::read_to_string(&head) {
            let raw = raw.trim();
            if let Some(r) = raw.strip_prefix("ref: refs/heads/") {
                return r.to_string();
            }
            // detached HEAD：给短哈希，总比空着强
            if let Some(sha) = raw.strip_prefix("ref: ") {
                return sha.rsplit('/').next().unwrap_or(sha).to_string();
            }
            return raw.chars().take(7).collect();
        }
        // .git 也可能是文件（worktree / submodule）：内容是 "gitdir: <path>"
        if let Ok(raw) = std::fs::read_to_string(dir.join(".git")) {
            if let Some(p) = raw.strip_prefix("gitdir: ") {
                let gd = std::path::Path::new(p.trim());
                let gd = if gd.is_absolute() { gd.to_path_buf() } else { dir.join(gd) };
                if let Ok(h) = std::fs::read_to_string(gd.join("HEAD")) {
                    let h = h.trim();
                    if let Some(r) = h.strip_prefix("ref: refs/heads/") {
                        return r.to_string();
                    }
                }
            }
        }
        cur = dir.parent();
    }
    String::new()
}

/// 构造模型后端。`None` 表示参数错误或环境不满足（已打印原因）。
///
/// 三种 provider 的定位不同：
/// - `deepseek`：真实模型，需要 `DEEPSEEK_API_KEY`
/// - `mock`：只回一句话，用于离线验证「装配 → 内核 → 沙箱 → 落盘」链路
/// - `selftest`：按脚本调用一次工具，用于离线验证「模型 → 工具 → 真实落盘」闭环
/// 构造模型**注册表**（多 provider，支持运行时切换）。
///
/// 与旧的 `build_model` 的区别：一次把所有可用 provider 都注册进去，
/// 于是 `/models` 能列出、能在会话中切换 —— 之前只有单个 provider，
/// 设置页只能把"服务商"标成只读。
///
/// **哪一个是默认**由 `--provider` 指定；`deepseek` 缺 key 时**不静默换成 mock**
/// （那会让用户以为在跟真模型聊），而是报错并提示替代方案。
fn build_models(provider: &str) -> Option<neo_core::models::ModelRegistry> {
    use neo_core::models::{ModelInfo, ModelRegistry};
    let mk = |name: &str, desc: &str, limit: u64, production: bool,
              p: Box<dyn neo_core::ModelProvider>| {
        (ModelInfo { name: name.into(), description: desc.into(), context_limit: limit, production }, p)
    };

    let mut entries: Vec<(ModelInfo, Box<dyn neo_core::ModelProvider>)> = Vec::new();

    // 真实模型：只在 key 可用时注册。缺 key 时不给一个"假 deepseek"条目 ——
    // 那会让 /models 列出一个切过去就报错的选项。
    let mut deepseek_ok = false;
    match neo_llm_deepseek::DeepSeekProvider::from_env() {
        Ok(p) => {
            entries.push(mk("deepseek", "DeepSeek chat-completions（真实模型）", 64_000, true, Box::new(p)));
            deepseek_ok = true;
        }
        Err(e) => {
            if provider == "deepseek" {
                eprintln!("[neo] {e}");
                eprintln!("       设置后重试：export DEEPSEEK_API_KEY=sk-...");
                eprintln!("       或离线试用：--provider mock | selftest");
                return None;
            }
        }
    }

    // 离线可用的桩：始终注册，便于随时对照（标 production=false，UI 可区分）
    entries.push(mk("mock", "确定性桩：只回一句话，不调真实模型", 0, false,
        Box::new(neo_llm_deepseek::ScriptedProvider::text_only(
            "（mock provider）本回答由确定性桩产生，未调用真实模型。"))));
    entries.push(mk("demo", "演示渲染：Markdown + 任务清单", 0, false,
        Box::new(neo_llm_deepseek::ScriptedProvider::demo().with_name("demo"))));
    entries.push(mk("selftest", "自检：按脚本调一次 apply_patch", 0, false,
        Box::new(neo_llm_deepseek::ScriptedProvider::scripted(
            vec![vec![neo_llm_deepseek::tool_call(
                "apply_patch",
                serde_json::json!({
                    "path": "selftest.txt",
                    "new": "由 selftest provider 经 apply_patch 写入。
",
                }),
            )]],
            "selftest 脚本执行完毕（工具是否成功见上方工具行与失败原因）。",
        )
        .with_name("selftest"))));

    // ── 用户级注册表里的服务商（providers.json）──────────────────────
    //
    // 只在**用户级**读取；项目级文件存在会被明确拒绝（安全敏感配置，
    // 见 neo-providers 的模块说明）。读失败要如实打印原因 ——
    // 静默忽略会让用户对着"配了却不生效"想不通。
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    match neo_providers::load(&cwd) {
        neo_providers::LoadOutcome::Loaded(reg) => {
            for e in &reg.providers {
                // 密钥取自 `api_key_env` 指名的环境变量；缺了跳过并提示，
                // 不给一个"切过去就连不上"的条目。
                let key = e
                    .api_key_env
                    .as_deref()
                    .and_then(|k| std::env::var(k).ok())
                    .filter(|v| !v.trim().is_empty());
                let Some(key) = key else {
                    let hint = e.api_key_env.as_deref().unwrap_or("(未声明 api_key_env)");
                    eprintln!("[neo] 服务商 {} 已跳过：环境变量 {hint} 未设置", e.name);
                    continue;
                };
                let p = neo_llm_deepseek::DeepSeekProvider {
                    api_key: key,
                    endpoint: e.base_url.clone().unwrap_or_default(),
                    path: "/chat/completions".to_string(),
                    model: e.model.clone().unwrap_or_else(|| "deepseek-chat".to_string()),
                    temperature: 0.0,
                    label: e.name.clone(),
                };
                let desc = e
                    .description
                    .clone()
                    .unwrap_or_else(|| "用户级 providers.json 注册".to_string());
                entries.push(mk(&e.name, &desc, e.context_limit, e.production, Box::new(p)));
            }
        }
        neo_providers::LoadOutcome::Absent => {}
        neo_providers::LoadOutcome::Failed(msg) => {
            eprintln!("[neo] providers.json 未生效：{msg}");
        }
    }

    // 校验默认项存在
    if !entries.iter().any(|(i, _)| i.name == provider) {
        let mut names: Vec<&str> = entries.iter().map(|(i, _)| i.name.as_str()).collect();
        names.sort();
        eprintln!(
            "[neo] 未知 provider：{provider}（可选 {}）{}",
            names.join(" | "),
            if !deepseek_ok { "；deepseek 需要 DEEPSEEK_API_KEY" } else { "" }
        );
        return None;
    }

    match ModelRegistry::new(provider, entries) {
        Ok(r) => Some(r),
        Err(e) => {
            eprintln!("[neo] 模型注册表构造失败：{e}");
            None
        }
    }
}

/// 旧的单 provider 构造（保留给需要"只有一个模型"的调用方）。
#[allow(dead_code)]
fn build_model(provider: &str) -> Option<Box<dyn neo_core::ModelProvider>> {
    match provider {
        "deepseek" => match neo_llm_deepseek::DeepSeekProvider::from_env() {
            Ok(p) => Some(Box::new(p)),
            Err(e) => {
                eprintln!("[neo] {e}");
                eprintln!("       设置后重试：export DEEPSEEK_API_KEY=sk-...");
                eprintln!("       或离线试用：--provider mock | selftest");
                None
            }
        },
        "mock" => Some(Box::new(neo_llm_deepseek::ScriptedProvider::text_only(
            "（mock provider）本回答由确定性桩产生，未调用真实模型。",
        ))),
        // 演示渲染（markdown + 任务清单），供人眼核对界面
        "demo" => Some(Box::new(neo_llm_deepseek::ScriptedProvider::demo())),
        "selftest" => Some(Box::new(neo_llm_deepseek::ScriptedProvider::scripted(
            vec![vec![neo_llm_deepseek::tool_call(
                "apply_patch",
                serde_json::json!({
                    "path": "selftest.txt",
                    "new": "由 selftest provider 经 apply_patch 写入。\n",
                }),
            )]],
            // 中性措辞：本 provider 不知道工具是否成功（可能被沙箱拦），
            // 断言"已落盘"会在被拦时给出与实际不符的输出。
            "selftest 脚本执行完毕（工具是否成功见上方工具行与失败原因）。",
        ))),
        other => {
            eprintln!("[neo] 未知 provider：{other}（可选 deepseek | mock | selftest | demo）");
            None
        }
    }
}
