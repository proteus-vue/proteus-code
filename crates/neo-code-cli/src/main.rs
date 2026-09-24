//! `neo` —— multitool 入口
//!
//! ```text
//! neo exec "<task>" [--mode <plan|confirm|default|auto-edit|full>] [--json] [--workspace <dir>]
//! neo serve          # Web 宿主（浏览器界面 + SSE 事件流）
//! neo app-server     # stdio JSON-RPC 宿主（编辑器/IDE/脚本接入）
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

mod tui_driver;
use tui_driver::KernelHandle;

const USAGE: &str = r#"neo —— 用 Rust 重构的编程 Agent 内核

用法：
  neo exec "<任务>" [选项]     无头跑一轮（真实模型 + 真实沙箱）
  neo serve [选项]             启动 Web 宿主（用提示打印的完整 URL 打开）
  neo app-server [选项]        启动 stdio JSON-RPC 宿主（编辑器/IDE 接入）
  neo [选项]                   启动 TUI 宿主（需真终端）
  neo --help | --version       查看用法 / 版本

  serve / desktop 的端点需要访问令牌：启动时会打印一条带令牌的完整 URL
  （形如 http://127.0.0.1:8787/#token=…），必须用它打开页面。令牌只在本次
  进程生命周期内有效。程序化客户端也可改用请求头 X-Neo-Token。

exec 选项：
  --mode <plan|confirm|default|auto-edit|full>   执行模式（默认 default）
  --workspace <dir>            工作区（默认当前目录）
  --max-steps <n>              步数上限（默认 16）
  --allow-writes               无人值守时自动批准写操作（默认拒绝）
  --goal <目标文本>            目标模式：按行拆子任务，自动逐阶段推进
                               直到完成或触发停止条件（与任务描述互斥）
  --json                       输出 JSON（便于脚本消费）
  --provider <deepseek|mock|demo|selftest|multitool|edit|usage>   模型后端（默认 deepseek）
                                selftest = 按脚本调用一次工具，验证完整链路（无需 key）

desktop 选项：
  --workspace <dir>            工作区（默认当前目录）
  --provider <...>             模型后端（同 serve）
  --mode <...>                 执行模式（同 serve）
  --egui                       用旧的原生窗口（egui，已冻结只修 bug）
  --webview                    用系统 webview 窗口
                               三种窗口实现，都不经 HTTP、不开端口：
                                 默认      = GPUI（NEO 的桌面 UI）
                                 --egui    = 旧实现（保留作回退通道）
                                 --webview = 系统 webview 指向内置 Web 宿主
                               窗口关闭即退出

serve 选项：
  --addr <host:port>           监听地址（默认 127.0.0.1:8787）
  --mode <...>                 执行模式（默认 default；Web 有交互审批，不需要放水）
  --workspace <dir>            工作区（默认当前目录）
  --provider <...>             模型后端（同 exec）

app-server 选项：
  --workspace <dir>            工作区（默认当前目录）
  --mode <...>                 执行模式（同 serve）
  --provider <...>             模型后端（同 exec）
  --listen <unix://path>       在本地 unix socket 上接多个客户端（共用一个内核）；
                               不给则走 stdin/stdout（一对一）。暂不支持 TCP。
  stdin/stdout 是协议通道（JSON-RPC 2.0，一行一条），诊断一律走 stderr。
  流程：先 initialize 握手 → turn/start 提交 → 事件以 event 通知到达 →
  需要审批时收到 approval_request 通知，用 approval/respond 应答同一 id →
  shutdown 或关闭 stdin 结束。方法清单见 initialize 响应里的 methods。

环境变量：
  DEEPSEEK_API_KEY             必需（provider=deepseek 时）
  DEEPSEEK_BASE_URL            可选，默认 api.deepseek.com
  DEEPSEEK_MODEL               可选，默认 deepseek-chat
"#;

/// `--addr` 是否只绑回环。
///
/// 只看主机部分：`127.0.0.0/8`、`::1`、`localhost` 算回环；`0.0.0.0`、`::`
/// 以及任何具体的外部地址都算对外可达。解析不出来时按"对外"处理 ——
/// 宁可多警告一次，也不要因为格式没料到而漏掉警告。
fn is_loopback_bind(bind: &str) -> bool {
    let host = match bind.rsplit_once(':') {
        // IPv6 字面量形如 [::1]:8787
        Some((h, _)) => h.trim_start_matches('[').trim_end_matches(']'),
        None => bind,
    };
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => false,
    }
}

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
        Some("app-server") => cmd_appserver(&args[1..]),
        #[cfg(feature = "desktop")]
        Some("desktop") => cmd_desktop(&args[1..]),
        #[cfg(not(feature = "desktop"))]
        Some("desktop") => {
            eprintln!("[neo] 本二进制未编译桌面宿主（构建时用了 --no-default-features）。");
            eprintln!("      需要桌面窗口请重装并保留默认 feature：cargo install neo-code-cli");
            2
        }
        Some("help") | Some("--help") | Some("-h") => {
            println!("{USAGE}");
            0
        }
        Some("version") | Some("--version") | Some("-V") => {
            println!("neo {}", env!("CARGO_PKG_VERSION"));
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
    // 默认空 = 自动选择：设置页配置的注册表服务商优先，其次 env deepseek
    // （选择逻辑见 build_models）。硬编码 "deepseek" 会让只配了智谱的用户
    // 直接 `neo` 时被要求 DEEPSEEK_API_KEY（真实反馈）。
    let mut provider = String::new();
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

    // 对外监听（非回环）会把工作区写操作暴露到网络上。令牌仍然拦得住
    // 未授权调用，但那是**明文 HTTP** —— 令牌与全部会话内容在网线上裸奔。
    // 因此必须显式警告，不能让它悄悄发生（默认值是回环，只有显式 --addr 才走到这）。
    if !is_loopback_bind(&bind) {
        eprintln!("[neo] ⚠️  --addr {bind} 不是回环地址 —— 服务将对外网络可达");
        eprintln!("       令牌能挡住未授权调用，但流量是明文 HTTP：");
        eprintln!("       令牌与会话内容都会在网络上可被嗅探。请只在可信网络使用，");
        eprintln!("       或置于带 TLS 的反向代理之后。");
    }

    let Some(models) = build_models(&provider) else {
        return 2;
    };
    // 横幅在内核装配（models 被移走）之后才打印，先取下实际选中的名字
    let model_name = models.current_provider().name().to_string();
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
    eprintln!("[neo] 模型   {model_name}");
    // 打印 page_url 而不是裸地址：端点需要访问令牌，令牌在 URL 的 fragment 里
    // （见 neo_host_web::auth）。用裸地址打开只会得到 401。
    eprintln!("[neo] Web 宿主 {}  （Ctrl-C 退出）", server.page_url());
    eprintln!("       必须用上面这条完整 URL 打开 —— 令牌在 # 之后，缺了会被 401 拒");
    eprintln!("       注意：事件流不重放，浏览器页面会先自动连上 SSE 再提交任务");
    // 内核线程在 op 通道关闭前不会退出，join 即"服务于请求直到进程结束"。
    let _ = kernel_thread.join();
    0
}

/// 启动 stdio JSON-RPC 宿主（`neo app-server`）—— 编辑器/IDE/脚本的通用入口。
///
/// 与 `serve` 的分工：`serve` 面向浏览器（回环 HTTP + SSE + 访问令牌），
/// `app-server` 面向**已有自己进程**的客户端（stdin/stdout 就是协议通道）。
/// 两者共用同一套内核装配与"内核独占线程"的模型，差别只在传输 ——
/// 所以这里不重复任何业务逻辑。
///
/// **stdout 只能出协议行**：本函数与内核都不得往 stdout 打印诊断，
/// 否则客户端解析失败（诊断一律 stderr）。
fn cmd_appserver(args: &[String]) -> i32 {
    let mut workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut provider = String::new();
    let mut mode = ExecMode::Default;
    let mut listen: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
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
            "--listen" => {
                i += 1;
                match args.get(i) {
                    Some(v) => match v.strip_prefix("unix://") {
                        // 只支持 unix socket：socket 文件的文件权限就是信任边界。
                        // TCP 要另做鉴权（同 `serve` 的访问令牌），不在本次范围。
                        Some(path) if !path.is_empty() => listen = Some(PathBuf::from(path)),
                        _ => {
                            eprintln!("[neo] --listen 只支持 unix://<路径>，收到：{v}");
                            eprintln!("      （TCP 需要鉴权，未做 —— 远程客户端走 `neo serve`）");
                            return 2;
                        }
                    },
                    None => {
                        eprintln!("[neo] --listen 需要一个 socket 路径（unix://<路径>）");
                        return 2;
                    }
                }
            }
            other => {
                eprintln!("[neo] app-server 不认识的参数：{other}");
                eprintln!("      它不从网络接收客户端（那是 `neo serve`），走 stdin/stdout 或 --listen unix://");
                return 2;
            }
        }
        i += 1;
    }

    let Some(models) = build_models(&provider) else {
        return 2;
    };
    let model_name = models.current_provider().name().to_string();
    let sandbox = Arc::new(neo_sandbox_local::LocalSandbox::new(&workspace));
    // 会话库：与 TUI/桌面同一目录（`.neo/sessions/`），app-server 不另开孤岛。
    // 起动会话用稳定 id `appserver`（可被 thread/resume 切走再切回）。
    let store = neo_session_store::SessionStore::open(workspace.join(".neo/sessions"));
    let start_id = "appserver".to_string();
    let persistence =
        Box::new(neo_session_local::JsonlPersistence::new(store.path_for(&start_id)));
    let opts = ExecOptions { mode, max_steps: 32, ..Default::default() };
    let mut kernel = build_kernel(&start_id, &workspace, &opts, models, sandbox, persistence);

    // 横幅走 stderr（stdout 是协议通道）。人在终端里直接跑时它说明"没卡住"；
    // 程序化客户端读 stdout，不受影响。
    match &listen {
        Some(socket) => eprintln!(
            "[neo] app-server 就绪：unix socket {}（多个客户端共用一个内核），诊断走 stderr",
            socket.display()
        ),
        None => eprintln!(
            "[neo] app-server 就绪：stdin/stdout 上跑 JSON-RPC 2.0（一行一条），诊断走 stderr"
        ),
    }
    eprintln!(
        "[neo] 工作区 {} · 模式 {} · 模型 {model_name}",
        workspace.display(),
        describe_mode(mode)
    );

    // 内核独占线程：Op 推进状态机；thread/* 操作会话库（切换要动同一个 Kernel）。
    // stdio 与 unix 两条传输共用这一个处理器 —— 差别只在"谁在连"，业务装配零重复。
    // PTY 会话表与内核同线程持有（Codex command/exec session 模式）。
    let mut sessions = neo_sandbox_local::SessionManager::default();
    let mut watch_state = WatchState::default();
    // on_bus 在 serve 启动时注入 EventBus（fs/watch 推 fs/changed 通知）
    let bus_slot: Arc<std::sync::Mutex<Option<neo_host_appserver::EventBus>>> =
        Arc::new(std::sync::Mutex::new(None));
    let bus_for_state = bus_slot.clone();
    let handle = {
        let bus_for_handle = bus_slot.clone();
        move |job| match job {
            neo_host_appserver::Job::Op(op) => {
                let events = kernel
                    .submit(op)
                    .unwrap_or_else(|e| vec![EventMsg::Error { message: e.to_string() }]);
                neo_host_appserver::JobOut::Events(events)
            }
            neo_host_appserver::Job::Thread { cmd, .. } => {
                watch_state.bus = bus_for_handle.clone();
                let res = thread_cmd(&mut kernel, &store, &mut sessions, cmd, &mut watch_state);
                neo_host_appserver::JobOut::Thread(res)
            }
        }
    };
    let result = match &listen {
        Some(socket) => neo_host_appserver::serve_unix_with_bus(socket, handle, move |bus| {
            *bus_for_state.lock().expect("bus 锁") = Some(bus);
        }),
        None => neo_host_appserver::serve_stdio_with_bus(handle, move |bus| {
            *bus_for_state.lock().expect("bus 锁") = Some(bus);
        }),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("[neo] app-server 传输失败：{e}");
            1
        }
    }
}


/// 内核工作区路径（git/info 的缺省 cwd）。
fn workspace_for_git(kernel: &neo_core::Kernel) -> std::path::PathBuf {
    kernel.cwd().to_path_buf()
}

fn events_from_logged(recs: &[neo_core::LoggedRecord]) -> Vec<EventMsg> {
    recs.iter()
        .filter(|r| r.kind == "event")
        .filter_map(|r| serde_json::from_value::<EventMsg>(r.payload.clone()).ok())
        .collect()
}

fn events_from_session(recs: &[neo_session::SessionEvent]) -> Vec<EventMsg> {
    recs.iter()
        .filter(|r| r.kind == "event")
        .filter_map(|r| serde_json::from_value::<EventMsg>(r.payload.clone()).ok())
        .collect()
}

fn load_events(
    kernel: &neo_core::Kernel,
    store: &neo_session_store::SessionStore,
    id: &str,
) -> Result<Vec<EventMsg>, String> {
    if id == kernel.session_id() {
        return Ok(events_from_logged(&kernel.log_for_test()));
    }
    if !store.exists(id) {
        return Err(format!("会话 {id} 不存在"));
    }
    let recs = neo_session::replay(&store.path_for(id))
        .map_err(|e| format!("读会话日志失败：{e}"))?;
    Ok(events_from_session(&recs))
}

/// `fs/*` 路径：绝对原样，相对按工作区解析。
fn fs_resolve(cwd: &std::path::Path, path: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    }
}

/// 标准 base64 解码（无换行）。失败返回 None。
fn b64_decode(s: &str) -> Option<Vec<u8>> {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = s.trim().to_string();
    while s.len() % 4 != 0 {
        s.push('=');
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        if chunk.len() < 4 {
            return None;
        }
        let mut n = 0u32;
        let mut pad = 0;
        for (i, &b) in chunk.iter().enumerate() {
            if b == b'=' {
                pad = 3 - i;
                n <<= 6;
                continue;
            }
            let v = T.iter().position(|&t| t == b)? as u32;
            n = (n << 6) | v;
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}

/// 工作区 git 元数据（**零子进程**：只读 `.git` 文件）。
///
/// 与 `detect_branch` 同一立场：启动/列举时不该为装饰性信息去 fork `git`，
/// 也不该假设用户装了 git。探测不到的字段留空串，客户端按"不在仓库里"显示。
fn git_info(ws: &std::path::Path) -> serde_json::Value {
    let mut root: Option<std::path::PathBuf> = None;
    let mut cur = Some(ws);
    while let Some(dir) = cur {
        if dir.join(".git").exists() {
            root = Some(dir.to_path_buf());
            break;
        }
        cur = dir.parent();
    }
    let Some(root) = root else {
        return serde_json::json!({
            "in_repo": false, "branch": "", "sha": "", "origin_url": "", "root": "",
        });
    };
    let gitdir_raw = root.join(".git");
    // worktree/submodule：`.git` 是文件，内容 "gitdir: <path>"
    let gitdir = if gitdir_raw.is_file() {
        std::fs::read_to_string(&gitdir_raw)
            .ok()
            .and_then(|s| {
                s.strip_prefix("gitdir:")
                    .map(|p| std::path::PathBuf::from(p.trim()))
            })
            .unwrap_or_else(|| gitdir_raw.clone())
    } else {
        gitdir_raw.clone()
    };
    // gitdir 可能是相对路径
    let gitdir = if gitdir.is_relative() {
        root.join(gitdir)
    } else {
        gitdir
    };

    let head = std::fs::read_to_string(gitdir.join("HEAD")).unwrap_or_default();
    let head = head.trim();
    let (branch, sha) = if let Some(r) = head.strip_prefix("ref: refs/heads/") {
        let branch = r.to_string();
        let ref_path = gitdir.join("refs/heads").join(&branch);
        let sha = std::fs::read_to_string(&ref_path)
            .map(|s| s.trim().chars().take(7).collect::<String>())
            .or_else(|_| {
                // packed-refs 兜底
                let packed = std::fs::read_to_string(gitdir.join("packed-refs")).unwrap_or_default();
                for line in packed.lines() {
                    if let Some(rest) = line.strip_prefix("#") {
                        let _ = rest;
                        continue;
                    }
                    let mut it = line.split_whitespace();
                    if let (Some(h), Some(name)) = (it.next(), it.next()) {
                        if name == format!("refs/heads/{branch}") {
                            return Ok(h.chars().take(7).collect::<String>());
                        }
                    }
                }
                Err(())
            })
            .unwrap_or_default();
        (branch, sha)
    } else if head.len() >= 7 {
        (String::new(), head.chars().take(7).collect())
    } else {
        (String::new(), String::new())
    };

    let config = std::fs::read_to_string(gitdir.join("config")).unwrap_or_default();
    let mut origin_url = String::new();
    let mut in_origin = false;
    for line in config.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_origin = l == "[remote \"origin\"]" || l.starts_with("[remote \"origin\"]");
            continue;
        }
        if in_origin && l.starts_with("url") {
            if let Some((_, v)) = l.split_once('=') {
                origin_url = v.trim().to_string();
            }
        }
    }

    serde_json::json!({
        "in_repo": true,
        "branch": branch,
        "sha": sha,
        "origin_url": origin_url,
        "root": root.display().to_string(),
    })
}

/// 把 Fact 列表渲染成可读 Markdown（`thread/export format=markdown`）。
fn facts_to_markdown(id: &str, items: &[neo_protocol::Fact]) -> String {
    use neo_protocol::Fact;
    let mut out = format!("# 会话 {id}\n\n");
    for f in items {
        match f {
            Fact::UserSaid(s) => {
                out.push_str("## 用户\n\n");
                out.push_str(s);
                out.push_str("\n\n");
            }
            Fact::AssistantSaid(s) | Fact::AssistantThought(s) => {
                out.push_str("## 助手\n\n");
                out.push_str(s);
                out.push_str("\n\n");
            }
            Fact::ToolFinished { name, exit_code, stdout, stderr, truncated, args } => {
                out.push_str(&format!("### 工具 `{name}` (exit {exit_code})\n\n"));
                if let Some(a) = args {
                    out.push_str(&format!("参数：`{a}`\n\n"));
                }
                if !stdout.is_empty() {
                    out.push_str("```\n");
                    out.push_str(stdout);
                    if *truncated {
                        out.push_str("\n…（已截断）\n");
                    }
                    out.push_str("```\n\n");
                }
                if !stderr.is_empty() {
                    out.push_str("stderr:\n\n```\n");
                    out.push_str(stderr);
                    out.push_str("```\n\n");
                }
            }
            Fact::Failed(s) => {
                out.push_str(&format!("**失败**：{s}\n\n"));
            }
            Fact::TurnFinished { input_tokens, output_tokens } => {
                out.push_str(&format!(
                    "---\n\n_本轮用量：input {input_tokens} / output {output_tokens}_\n\n"
                ));
            }
            Fact::PatchPreview { path, .. } => {
                out.push_str(&format!("### 改动预览 `{path}`\n\n"));
            }
            Fact::ApprovalNeeded { detail } => {
                out.push_str(&format!("**待审批**：{detail}\n\n"));
            }
            Fact::TodoList(items) => {
                out.push_str("### 任务清单\n\n");
                for it in items {
                    out.push_str(&format!("- [{}] {}\n", it.status as u8, it.content));
                }
                out.push_str("\n");
            }
            Fact::RefsResolved(lines) => {
                out.push_str(&format!("> 引用：{}\n\n", lines.join("；")));
            }
            Fact::InstructionsLoaded { sources, truncated } => {
                out.push_str(&format!(
                    "> 指令：{}{}\n\n",
                    sources.join("，"),
                    if *truncated { "（已截断）" } else { "" }
                ));
            }
            Fact::SessionReady { session_id } => {
                out.push_str(&format!("_会话 {session_id} 就绪_\n\n"));
            }
            Fact::ModelSwitched { model, context_limit } => {
                out.push_str(&format!("_模型 → {model}（窗口 {context_limit}）_\n\n"));
            }
            Fact::ContextCompacted { removed_messages } => {
                out.push_str(&format!("_上下文已压缩（移除 {removed_messages} 条）_\n\n"));
            }
            Fact::Rewound { turns, removed_messages, files_kept } => {
                out.push_str(&format!(
                    "_已回退 {turns} 轮（删 {removed_messages} 条，保留 {files_kept} 个文件改动）_\n\n"
                ));
            }
            Fact::FilesChanged(files) => {
                out.push_str("### 已修改文件\n\n");
                for f in files {
                    out.push_str(&format!("- `{}` +{} -{}\n", f.path, f.additions, f.deletions));
                }
                out.push_str("\n");
            }
            Fact::Goal(s) | Fact::GoalCleared(s) => {
                out.push_str(&format!("> 目标：{s}\n\n"));
            }
        }
    }
    out
}

/// `thread/*` 的装配层实现：会话库 + 内核切换。
///
/// 与 TUI/桌面的 `Sessions` 同一套语义，但那边要经 `KernelHandle` 跨线程
/// 查询，这边内核就在本闭包里 —— 直接调 `switch_session`，不经句柄。
/// 判据保持一致：不存在的 id 报错、不能删当前会话、切换后把历史事件交出去。
fn thread_cmd(
    kernel: &mut neo_core::Kernel,
    store: &neo_session_store::SessionStore,
    sessions: &mut neo_sandbox_local::SessionManager,
    cmd: neo_host_appserver::ThreadCmd,
    watch_state: &mut WatchState,
) -> neo_host_appserver::ThreadResult {
    use neo_host_appserver::{ThreadCmd, ThreadResult};

    fn summary(m: neo_session_store::SessionMeta) -> serde_json::Value {
        let (additions, deletions) = m.changes.unwrap_or((0, 0));
        let updated_ms = std::fs::metadata(&m.path)
            .and_then(|md| md.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        serde_json::json!({
            "id": m.id,
            "title": m.title,
            "has_title": m.has_title(),
            "records": m.records,
            "bytes": m.bytes,
            "additions": additions,
            "deletions": deletions,
            "has_changes": m.changes.is_some(),
            "updated_ms": updated_ms,
            "state": match m.state {
                neo_session::SessionState::Idle => "idle",
                neo_session::SessionState::Failed => "failed",
                neo_session::SessionState::Interrupted => "interrupted",
                neo_session::SessionState::Empty => "empty",
            },
            "archived": m.archived,
        })
    }

    fn history_of(k: &mut neo_core::Kernel) -> Vec<EventMsg> {
        k.log_for_test()
            .into_iter()
            .filter(|rec| rec.kind == "event")
            .filter_map(|rec| serde_json::from_value::<EventMsg>(rec.payload).ok())
            .collect()
    }

    match cmd {
        ThreadCmd::List => {
            let threads: Vec<serde_json::Value> =
                store.list().into_iter().map(summary).collect();
            ThreadResult::Value(serde_json::json!({ "threads": threads }))
        }
        ThreadCmd::Get { id } => match store.list().into_iter().find(|m| m.id == id) {
            Some(m) => ThreadResult::Value(serde_json::json!({ "thread": summary(m) })),
            None => ThreadResult::Error(format!("会话 {id} 不存在")),
        },
        ThreadCmd::Resume { id } => {
            if !store.exists(&id) {
                return ThreadResult::Error(format!("会话 {id} 不存在"));
            }
            let p = Box::new(neo_session_local::JsonlPersistence::new(store.path_for(&id)));
            kernel.switch_session(&id, p);
            let history = history_of(kernel);
            ThreadResult::Resumed {
                result: serde_json::json!({ "id": id, "events_replayed": history.len() }),
                history,
            }
        }
        ThreadCmd::Create => {
            let id = store.new_id();
            let p = Box::new(neo_session_local::JsonlPersistence::new(store.path_for(&id)));
            kernel.switch_session(&id, p);
            ThreadResult::Value(serde_json::json!({ "id": id }))
        }
        ThreadCmd::Delete { id } => {
            if id == kernel.session_id() {
                return ThreadResult::Error("不能删除当前正在使用的会话（先 thread/resume 到别的会话）".into());
            }
            match store.delete(&id) {
                Ok(removed) => ThreadResult::Value(serde_json::json!({ "removed": removed })),
                Err(e) => ThreadResult::Error(e.to_string()),
            }
        }
        ThreadCmd::Rename { id, title } => {
            // create 是惰性建文件：刚 create 的当前会话可能还没 JSONL。
            // 非当前且文件不存在 → 明确报不存在（不能凭 rename 凭空造会话）。
            let is_current = id == kernel.session_id();
            if !store.exists(&id) && !is_current {
                return ThreadResult::Error(format!("会话 {id} 不存在"));
            }
            match store.set_title(&id, &title) {
                Ok(_) => {
                    // 回读摘要：新建文件后 list 应能看见
                    let m = store.list().into_iter().find(|m| m.id == id);
                    match m {
                        Some(m) => ThreadResult::Value(serde_json::json!({
                            "id": m.id,
                            "title": m.title,
                            "has_title": m.has_title(),
                        })),
                        None => ThreadResult::Error(format!("会话 {id} 不存在")),
                    }
                }
                Err(e) => ThreadResult::Error(e.to_string()),
            }
        }
        ThreadCmd::Tools => {
            let tools: Vec<serde_json::Value> = kernel
                .tool_schemas()
                .into_iter()
                .map(|s| {
                    serde_json::json!({
                        "name": s.name,
                        "description": s.description,
                        "parameters": s.parameters,
                    })
                })
                .collect();
            ThreadResult::Value(serde_json::json!({ "tools": tools }))
        }
        ThreadCmd::Models => {
            let current = kernel.current_model().to_string();
            let models: Vec<serde_json::Value> = kernel
                .available_models()
                .into_iter()
                .map(|m| {
                    serde_json::json!({
                        "name": m.name,
                        "description": m.description,
                        "context_limit": m.context_limit,
                        "production": m.production,
                        "current": m.name == current,
                    })
                })
                .collect();
            ThreadResult::Value(serde_json::json!({ "models": models, "current": current }))
        }
        ThreadCmd::GitInfo { cwd } => {
            let ws = cwd
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| workspace_for_git(kernel));
            ThreadResult::Value(git_info(&ws))
        }
        ThreadCmd::Export { id, format } => {
            let id = id.unwrap_or_else(|| kernel.session_id().to_string());
            if !store.exists(&id) {
                return ThreadResult::Error(format!("会话 {id} 不存在"));
            }
            let path = store.path_for(&id);
            let events: Vec<EventMsg> = match neo_session::replay(&path) {
                Ok(recs) => recs
                    .into_iter()
                    .filter(|r| r.kind == "event")
                    .filter_map(|r| serde_json::from_value::<EventMsg>(r.payload).ok())
                    .collect(),
                Err(e) => return ThreadResult::Error(format!("读会话日志失败：{e}")),
            };
            let items = neo_protocol::facts_of(&events);
            let fmt = format.as_deref().unwrap_or("markdown");
            match fmt {
                "markdown" | "md" => ThreadResult::Value(serde_json::json!({
                    "id": id,
                    "format": "markdown",
                    "content": facts_to_markdown(&id, &items),
                })),
                "json" => ThreadResult::Value(serde_json::json!({
                    "id": id,
                    "format": "json",
                    "events_replayed": events.len(),
                    "items": items,
                })),
                other => ThreadResult::Error(format!(
                    "不支持的导出格式：{other}（可选 markdown / json）"
                )),
            }
        }
        ThreadCmd::History { id } => {
            // 只读投影，**不切换**：侧栏预览别的会话不该打断当前对话。
            let id = id.unwrap_or_else(|| kernel.session_id().to_string());
            if !store.exists(&id) {
                return ThreadResult::Error(format!("会话 {id} 不存在"));
            }
            let path = store.path_for(&id);
            let events: Vec<EventMsg> = match neo_session::replay(&path) {
                Ok(recs) => recs
                    .into_iter()
                    .filter(|r| r.kind == "event")
                    .filter_map(|r| serde_json::from_value::<EventMsg>(r.payload).ok())
                    .collect(),
                Err(e) => return ThreadResult::Error(format!("读会话日志失败：{e}")),
            };
            // 与 T6 同源：投影就是 facts_of，没有第二套判定
            let items = neo_protocol::facts_of(&events);
            ThreadResult::Value(serde_json::json!({
                "id": id,
                "events_replayed": events.len(),
                "items": items,
            }))
        }
        ThreadCmd::Archive { id, archived } => {
            if !store.exists(&id) {
                return ThreadResult::Error(format!("会话 {id} 不存在"));
            }
            match store.set_archived(&id, archived) {
                Ok(_) => ThreadResult::Value(serde_json::json!({
                    "id": id,
                    "archived": archived,
                })),
                Err(e) => ThreadResult::Error(e.to_string()),
            }
        }
        ThreadCmd::HooksList => {
            // 诚实边界：尚无 hooks 系统 —— 空列表而不是编造条目
            ThreadResult::Value(serde_json::json!({ "hooks": [] }))
        }
        ThreadCmd::McpServerStatusList => {
            let specs = neo_mcp::config::load_user_config().unwrap_or_else(|_| vec![]);
            let tools: Vec<String> = kernel
                .tool_schemas()
                .into_iter()
                .filter(|t| t.name.starts_with("mcp__"))
                .map(|t| t.name.clone())
                .collect();
            let servers: Vec<serde_json::Value> = specs
                .iter()
                .map(|s| {
                    let prefix = format!("mcp__{}__", s.name.replace('-', "_"));
                    let n = tools.iter().filter(|t| t.starts_with(&prefix)).count();
                    serde_json::json!({
                        "name": s.name,
                        "transport": if s.command.is_empty() { "http" } else { "stdio" },
                        "tools_registered": n,
                        "status": if n > 0 { "connected" } else { "configured" },
                    })
                })
                .collect();
            ThreadResult::Value(serde_json::json!({ "servers": servers }))
        }
        ThreadCmd::PermissionProfileList => {
            let current = kernel.exec_mode();
            let profiles: Vec<serde_json::Value> = [
                ("plan", "只读计划"),
                ("confirm_before", "变更前确认"),
                ("default", "默认"),
                ("auto_edit", "自动编辑"),
                ("full_access", "完全访问"),
            ]
            .iter()
            .map(|(id, label)| {
                serde_json::json!({
                    "id": id,
                    "label": label,
                    "current": serde_json::to_value(current)
                        .ok()
                        .and_then(|v| v.as_str().map(|s| s == *id))
                        .unwrap_or(false),
                })
            })
            .collect();
            ThreadResult::Value(serde_json::json!({ "profiles": profiles }))
        }
        ThreadCmd::ModelProviderCapabilities => {
            let current = kernel.current_model().to_string();
            let models: Vec<serde_json::Value> = kernel
                .available_models()
                .into_iter()
                .map(|m| {
                    serde_json::json!({
                        "name": m.name,
                        "description": m.description,
                        "context_limit": m.context_limit,
                        "production": m.production,
                        "current": m.name == current,
                    })
                })
                .collect();
            ThreadResult::Value(serde_json::json!({
                "current": current,
                "models": models,
            }))
        }
        ThreadCmd::FsCopy { from, to } => {
            let src = fs_resolve(kernel.cwd(), &from);
            let dst = fs_resolve(kernel.cwd(), &to);
            match std::fs::copy(&src, &dst) {
                Ok(bytes) => ThreadResult::Value(serde_json::json!({
                    "from": src.display().to_string(),
                    "to": dst.display().to_string(),
                    "bytes": bytes,
                })),
                Err(e) => ThreadResult::Error(format!(
                    "复制失败：{} → {}（{e}）",
                    src.display(),
                    dst.display()
                )),
            }
        }
        ThreadCmd::FsCreateDirectory { path } => {
            let full = fs_resolve(kernel.cwd(), &path);
            match std::fs::create_dir_all(&full) {
                Ok(()) => ThreadResult::Value(serde_json::json!({
                    "path": full.display().to_string(),
                    "created": true,
                })),
                Err(e) => ThreadResult::Error(format!("建目录失败：{}（{e}）", full.display())),
            }
        }
        ThreadCmd::FsRemove { path } => {
            let full = fs_resolve(kernel.cwd(), &path);
            let meta = match std::fs::metadata(&full) {
                Ok(m) => m,
                Err(e) => {
                    return ThreadResult::Error(format!("删除失败：{}（{e}）", full.display()))
                }
            };
            let res = if meta.is_dir() {
                std::fs::remove_dir_all(&full)
            } else {
                std::fs::remove_file(&full)
            };
            match res {
                Ok(()) => ThreadResult::Value(serde_json::json!({
                    "path": full.display().to_string(),
                    "removed": true,
                })),
                Err(e) => ThreadResult::Error(format!("删除失败：{}（{e}）", full.display())),
            }
        }
        ThreadCmd::InjectItems { text } => {
            match kernel.inject_user_text(&text) {
                Ok(()) => ThreadResult::Value(serde_json::json!({
                    "id": kernel.session_id(),
                    "injected": true,
                    "chars": text.chars().count(),
                })),
                Err(e) => ThreadResult::Error(e.to_string()),
            }
        }
        ThreadCmd::Revert { before_turn_id } => {
            // beforeTurnId 形如 turn-N（1 基）。rewind 步数 = 总轮数 - (N-1)
            let n: usize = before_turn_id
                .strip_prefix("turn-")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            if n == 0 {
                return ThreadResult::Error(format!(
                    "无法解析 beforeTurnId「{before_turn_id}」（期望 turn-N）"
                ));
            }
            let total = kernel.user_turn_count();
            let turns = total.saturating_sub(n.saturating_sub(1));
            if turns == 0 {
                return ThreadResult::Value(serde_json::json!({
                    "before": before_turn_id,
                    "turns": 0,
                    "message": "该轮之前没有可回退内容",
                }));
            }
            match kernel.submit(neo_protocol::Op::Rewind { turns }) {
                Ok(events) => ThreadResult::Resumed {
                    result: serde_json::json!({
                        "before": before_turn_id,
                        "turns": turns,
                        "events": events.len(),
                    }),
                    history: events,
                },
                Err(e) => ThreadResult::Error(e.to_string()),
            }
        }
        ThreadCmd::MetadataUpdate { id, branch, sha, origin_url } => {
            if id != kernel.session_id() && !store.exists(&id) {
                return ThreadResult::Error(format!("会话 {id} 不存在"));
            }
            let mut payload = serde_json::json!({ "git_info": {} });
            if let Some(obj) = payload.get_mut("git_info").and_then(|v| v.as_object_mut()) {
                if let Some(b) = branch {
                    obj.insert("branch".into(), serde_json::Value::String(b));
                }
                if let Some(s) = sha {
                    obj.insert("sha".into(), serde_json::Value::String(s));
                }
                if let Some(o) = origin_url {
                    obj.insert("origin_url".into(), serde_json::Value::String(o));
                }
            }
            match store.append_op(&id, payload) {
                Ok(_) => ThreadResult::Value(serde_json::json!({ "id": id, "updated": true })),
                Err(e) => ThreadResult::Error(e.to_string()),
            }
        }
        ThreadCmd::AttachmentAdd { id, attachment_type, identity_key, payload } => {
            if id != kernel.session_id() && !store.exists(&id) {
                return ThreadResult::Error(format!("会话 {id} 不存在"));
            }
            match store.upsert_attachment(&id, &attachment_type, &identity_key, payload) {
                Ok(entry) => ThreadResult::Value(entry),
                Err(e) => ThreadResult::Error(e.to_string()),
            }
        }
        ThreadCmd::AttachmentList { id } => {
            let id = if id == kernel.session_id() || store.exists(&id) {
                id
            } else {
                return ThreadResult::Error(format!("会话 {id} 不存在"));
            };
            ThreadResult::Value(serde_json::json!({
                "id": id,
                "attachments": store.list_attachments(&id),
            }))
        }
        ThreadCmd::AttachmentRemove { id, attachment_type, identity_key } => {
            let removed = store.remove_attachment(&id, &attachment_type, &identity_key);
            if !removed {
                return ThreadResult::Error(format!(
                    "附件不存在：{attachment_type}/{identity_key}"
                ));
            }
            ThreadResult::Value(serde_json::json!({
                "id": id,
                "attachmentType": attachment_type,
                "identityKey": identity_key,
                "removed": true,
            }))
        }
        ThreadCmd::McpToolCall { server, tool, arguments } => {
            let name = format!(
                "mcp__{}__{}",
                server.replace('-', "_"),
                tool.replace('-', "_")
            );
            let out = kernel.execute_tool_by_name(&name, &arguments);
            if out.exit_code != 0 {
                return ThreadResult::Error(if out.stderr.is_empty() {
                    format!("MCP 工具失败：{name}")
                } else {
                    out.stderr
                });
            }
            ThreadResult::Value(serde_json::json!({
                "server": server,
                "tool": tool,
                "output": out.stdout,
                "truncated": out.truncated,
            }))
        }
        ThreadCmd::McpResourceRead { server, uri } => {
            let name = format!("mcp__{}__read_resource", server.replace('-', "_"));
            let out = kernel.execute_tool_by_name(&name, &serde_json::json!({ "uri": uri }));
            if out.exit_code != 0 {
                return ThreadResult::Error(if out.stderr.is_empty() {
                    format!("MCP 资源读取失败：{name}")
                } else {
                    out.stderr
                });
            }
            ThreadResult::Value(serde_json::json!({
                "server": server,
                "uri": uri,
                "text": out.stdout,
                "truncated": out.truncated,
            }))
        }
        ThreadCmd::GoalGet => match kernel.goal_snapshot() {
            Some(snap) => ThreadResult::Value(serde_json::json!({ "goal": snap })),
            None => ThreadResult::Value(serde_json::json!({ "goal": null })),
        },
        ThreadCmd::NameSet { id, title } => {
            let is_current = id == kernel.session_id();
            if !store.exists(&id) && !is_current {
                return ThreadResult::Error(format!("会话 {id} 不存在"));
            }
            match store.set_title(&id, &title) {
                Ok(_) => ThreadResult::Value(serde_json::json!({
                    "id": id,
                    "title": title,
                    "has_title": true,
                })),
                Err(e) => ThreadResult::Error(e.to_string()),
            }
        }
        ThreadCmd::ItemsList { id, limit } => {
            let id = id.unwrap_or_else(|| kernel.session_id().to_string());
            let events = match load_events(kernel, store, &id) {
                Ok(e) => e,
                Err(msg) => return ThreadResult::Error(msg),
            };
            let mut items: Vec<serde_json::Value> =
                neo_protocol::facts_of(&events).into_iter().map(|f| serde_json::to_value(f).unwrap_or(serde_json::Value::Null)).collect();
            if let Some(n) = limit {
                items.truncate(n);
            }
            ThreadResult::Value(serde_json::json!({
                "id": id,
                "items": items,
            }))
        }
        ThreadCmd::TurnsList { id, limit } => {
            let id = id.unwrap_or_else(|| kernel.session_id().to_string());
            let events = match load_events(kernel, store, &id) {
                Ok(e) => e,
                Err(msg) => return ThreadResult::Error(msg),
            };
            // 用户轮 = UserSubmitted；轮摘要 = 其后第一条 AgentMessageDone 截断
            let mut turns: Vec<serde_json::Value> = Vec::new();
            let mut cur: Option<serde_json::Value> = None;
            for ev in &events {
                match &ev {
                    EventMsg::UserSubmitted { text } => {
                        if let Some(t) = cur.take() {
                            turns.push(t);
                        }
                        cur = Some(serde_json::json!({
                            "id": format!("turn-{}", turns.len() + 1),
                            "user": text.chars().take(200).collect::<String>(),
                            "assistant": "",
                        }));
                    }
                    EventMsg::AgentMessageDone { text } => {
                        if let Some(t) = cur.as_mut() {
                            if let Some(obj) = t.as_object_mut() {
                                obj.insert("assistant".into(), serde_json::Value::String(
                                    text.chars().take(400).collect::<String>(),
                                ));
                            }
                        }
                    }
                    _ => {}
                }
            }
            if let Some(t) = cur.take() {
                turns.push(t);
            }
            if let Some(n) = limit {
                turns.truncate(n);
            }
            ThreadResult::Value(serde_json::json!({ "id": id, "turns": turns }))
        }
        ThreadCmd::SkillsList => {
            let skills: Vec<serde_json::Value> = kernel
                .skill_list()
                .into_iter()
                .map(|(name, desc)| serde_json::json!({ "name": name, "description": desc }))
                .collect();
            ThreadResult::Value(serde_json::json!({ "skills": skills }))
        }
        ThreadCmd::ConfigRead => {
            let res = kernel.resolution();
            ThreadResult::Value(serde_json::json!({
                "exec_mode": serde_json::to_value(kernel.exec_mode()).unwrap_or(serde_json::Value::Null),
                "sandbox_mode": serde_json::to_value(res.sandbox).unwrap_or(serde_json::Value::Null),
                "approval_policy": serde_json::to_value(res.approval).unwrap_or(serde_json::Value::Null),
                "file_edit": match res.file_edit { neo_config::FileEditPolicy::Auto => "auto", neo_config::FileEditPolicy::Ask => "ask" },
                "model": kernel.current_model(),
                "workspace": kernel.cwd().display().to_string(),
                "session_id": kernel.session_id(),
                "token_budget": kernel.session_token_budget().0,
                "protocol_version": neo_host_appserver::PROTOCOL_VERSION,
            }))
        }
        ThreadCmd::FsReadFile { path } => {
            let full = fs_resolve(kernel.cwd(), &path);
            match std::fs::read(&full) {
                Ok(raw) => {
                    // 有界：与工具输出同量级上限
                    const CAP: usize = 2 * 1024 * 1024;
                    let truncated = raw.len() > CAP;
                    let slice = if truncated { &raw[..CAP] } else { &raw[..] };
                    ThreadResult::Value(serde_json::json!({
                        "path": full.display().to_string(),
                        "bytes": raw.len(),
                        "truncated": truncated,
                        "content": String::from_utf8_lossy(slice),
                    }))
                }
                Err(e) => ThreadResult::Error(format!("读取失败：{}（{e}）", full.display())),
            }
        }
        ThreadCmd::FsGetMetadata { path } => {
            let full = fs_resolve(kernel.cwd(), &path);
            match std::fs::metadata(&full) {
                Ok(md) => ThreadResult::Value(serde_json::json!({
                    "path": full.display().to_string(),
                    "is_dir": md.is_dir(),
                    "is_file": md.is_file(),
                    "bytes": md.len(),
                })),
                Err(e) => ThreadResult::Error(format!("元数据失败：{}（{e}）", full.display())),
            }
        }
        ThreadCmd::FsReadDirectory { path } => {
            let full = fs_resolve(kernel.cwd(), &path);
            match std::fs::read_dir(&full) {
                Ok(rd) => {
                    let mut names: Vec<String> = Vec::new();
                    for e in rd.flatten() {
                        names.push(e.file_name().to_string_lossy().into_owned());
                    }
                    names.sort();
                    ThreadResult::Value(serde_json::json!({
                        "path": full.display().to_string(),
                        "entries": names,
                    }))
                }
                Err(e) => ThreadResult::Error(format!("列目录失败：{}（{e}）", full.display())),
            }
        }
        ThreadCmd::FsWriteFile { path, data, data_base64 } => {
            let full = fs_resolve(kernel.cwd(), &path);
            let content = match data_base64 {
                Some(b64) => {
                    // 简单 base64 解码（标准 alphabet）
                    match b64_decode(&b64) {
                        Some(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                        None => return ThreadResult::Error("data_base64 解码失败".into()),
                    }
                }
                None => data,
            };
            let mode = kernel.sandbox_mode();
            match kernel.sandbox().write_file(mode, &full, &content) {
                neo_core::FileOutcome::Written { bytes } => ThreadResult::Value(
                    serde_json::json!({ "path": full.display().to_string(), "bytes": bytes }),
                ),
                neo_core::FileOutcome::Denied { reason } => ThreadResult::Error(reason),
                neo_core::FileOutcome::Failed { reason } => ThreadResult::Error(reason),
            }
        }
        ThreadCmd::ExecStart { command, cols, rows } => {
            let mode = kernel.resolution().sandbox;
            let cwd = kernel.cwd().to_path_buf();
            // LocalSandbox::supports 语义：全权限 always；受限档看平台
            let sb = neo_sandbox_local::LocalSandbox::new(&cwd);
            let supported = neo_core::SandboxBackend::supports(&sb, mode);
            match sessions.start(mode, &cwd, &command, cols, rows, supported) {
                Ok(s) => {
                    let id = s.session_id.clone();
                    ThreadResult::Resumed {
                        result: serde_json::json!({
                            "session_id": s.session_id,
                            "running": s.running,
                            "output": s.output,
                            "exit_code": s.exit_code,
                            "truncated": s.truncated,
                            "mode": "pty",
                        }),
                        history: vec![EventMsg::ToolCallBegin {
                            id: id.clone(),
                            name: "bash".into(),
                            arguments: serde_json::json!({ "cmd": command, "session": true }),
                        }],
                    }
                }
                Err(e) => ThreadResult::Error(e),
            }
        }
        ThreadCmd::ExecWrite { session_id, data } => match sessions.write(&session_id, &data) {
            Ok(s) => ThreadResult::Value(serde_json::json!({
                "session_id": s.session_id,
                "output": s.output,
                "running": s.running,
                "exit_code": s.exit_code,
                "truncated": s.truncated,
            })),
            Err(e) => ThreadResult::Error(e),
        },
        ThreadCmd::ExecResize { session_id, cols, rows } => {
            match sessions.resize(&session_id, cols, rows) {
                Ok(()) => ThreadResult::Value(serde_json::json!({
                    "session_id": session_id,
                    "cols": cols,
                    "rows": rows,
                    "resized": true,
                })),
                Err(e) => ThreadResult::Error(e),
            }
        }
        ThreadCmd::ExecTerminate { session_id } => match sessions.terminate(&session_id) {
            Ok(s) => ThreadResult::Value(serde_json::json!({
                "session_id": s.session_id,
                "terminated": true,
                "output": s.output,
                "exit_code": s.exit_code,
                "running": s.running,
            })),
            Err(e) => ThreadResult::Error(e),
        },
        // 本地插件市场：只碰 $NEO_HOME 磁盘配置与资源复制，无账号、不执行插件内容。
        ThreadCmd::MarketplaceAdd { name, source } => {
            match neo_skill_loader::marketplace::marketplace_add(&name, &source) {
                Ok(v) => ThreadResult::Value(v),
                Err(e) => ThreadResult::Error(e),
            }
        }
        ThreadCmd::MarketplaceRemove { name } => {
            match neo_skill_loader::marketplace::marketplace_remove(&name) {
                Ok(removed) => ThreadResult::Value(serde_json::json!({ "name": name, "removed": removed })),
                Err(e) => ThreadResult::Error(e),
            }
        }
        ThreadCmd::MarketplaceUpgrade { name } => {
            match neo_skill_loader::marketplace::marketplace_upgrade(name.as_deref()) {
                Ok(v) => ThreadResult::Value(v),
                Err(e) => ThreadResult::Error(e),
            }
        }
        ThreadCmd::PluginList => match neo_skill_loader::marketplace::plugin_list() {
            Ok(v) => ThreadResult::Value(v),
            Err(e) => ThreadResult::Error(e),
        },
        ThreadCmd::PluginInstalled => match neo_skill_loader::marketplace::plugin_installed() {
            Ok(v) => ThreadResult::Value(v),
            Err(e) => ThreadResult::Error(e),
        },
        ThreadCmd::PluginReconcile => match neo_skill_loader::marketplace::plugin_reconcile() {
            Ok(v) => ThreadResult::Value(v),
            Err(e) => ThreadResult::Error(e),
        },
        ThreadCmd::PluginRead { name } => match neo_skill_loader::marketplace::plugin_read(&name) {
            Ok(v) => ThreadResult::Value(v),
            Err(e) => ThreadResult::Error(e),
        },
        ThreadCmd::PluginInstall { name } => {
            match neo_skill_loader::marketplace::plugin_install(&name) {
                Ok(v) => ThreadResult::Value(v),
                Err(e) => ThreadResult::Error(e),
            }
        }
        ThreadCmd::PluginUninstall { id } => {
            match neo_skill_loader::marketplace::plugin_uninstall(&id) {
                Ok(removed) => ThreadResult::Value(serde_json::json!({ "pluginId": id, "removed": removed })),
                Err(e) => ThreadResult::Error(e),
            }
        }
        ThreadCmd::PluginSkillRead { plugin, skill } => {
            match neo_skill_loader::marketplace::plugin_skill_read(&plugin, &skill) {
                Ok(v) => ThreadResult::Value(v),
                Err(e) => ThreadResult::Error(e),
            }
        }
        // ── 线分区（sidecar，不进会话 JSONL）────────────────────────────
        ThreadCmd::SectionList { cursor: _, limit } => {
            let mut sections = store.list_sections();
            if let Some(n) = limit {
                sections.truncate(n as usize);
            }
            ThreadResult::Value(serde_json::json!({
                "sections": sections,
                "nextCursor": serde_json::Value::Null,
            }))
        }
        ThreadCmd::SectionCreate { name, appearance } => match store.create_section(&name, appearance) {
            Ok(v) => ThreadResult::Value(v),
            Err(e) => ThreadResult::Error(e.to_string()),
        },
        ThreadCmd::SectionUpdate { section_id, name, appearance } => {
            match store.update_section(&section_id, &name, appearance) {
                Ok(v) => ThreadResult::Value(v),
                Err(e) => ThreadResult::Error(e.to_string()),
            }
        }
        ThreadCmd::SectionDelete { section_id } => match store.delete_section(&section_id) {
            Ok(removed) => ThreadResult::Value(serde_json::json!({
                "sectionId": section_id,
                "removed": removed,
            })),
            Err(e) => ThreadResult::Error(e.to_string()),
        },
        ThreadCmd::SectionMoveThread { thread_id, section_id, before_thread_id } => {
            if thread_id != kernel.session_id() && !store.exists(&thread_id) {
                return ThreadResult::Error(format!("会话 {thread_id} 不存在"));
            }
            match store.move_thread_to_section(&thread_id, section_id.as_deref(), before_thread_id.as_deref())
            {
                Ok(v) => ThreadResult::Value(v),
                Err(e) => ThreadResult::Error(e.to_string()),
            }
        }
        ThreadCmd::ThreadUnsubscribe { id } => ThreadResult::Value(store.unsubscribe(&id)),
        // ── 配置写：落 $NEO_HOME/config.json（本仓无 config.toml 装载器）──
        ThreadCmd::ConfigValueWrite {
            key_path,
            value,
            merge_strategy,
            file_path,
            expected_version,
        } => {
            match config_write_single(&key_path, &value, &merge_strategy, file_path.as_deref(), expected_version.as_deref()) {
                Ok(v) => ThreadResult::Value(v),
                Err(e) => ThreadResult::Error(e),
            }
        }
        ThreadCmd::ConfigBatchWrite {
            edits,
            file_path,
            expected_version,
            reload_user_config,
        } => match config_write_batch(&edits, file_path.as_deref(), expected_version.as_deref()) {
            Ok(mut v) => {
                if reload_user_config.unwrap_or(false) {
                    let n = neo_mcp::config::load_user_config().map(|s| s.len()).unwrap_or(0);
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert("mcpServersReloaded".into(), serde_json::json!(n));
                    }
                }
                ThreadResult::Value(v)
            }
            Err(e) => ThreadResult::Error(e),
        },
        ThreadCmd::ConfigMcpReload => match neo_mcp::config::load_user_config() {
            Ok(specs) => {
                let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
                ThreadResult::Value(serde_json::json!({
                    "reloaded": true,
                    "servers": names,
                    "note": "已重读 $NEO_HOME/mcp.json；已注册工具需重启会话才换血",
                }))
            }
            Err(e) => ThreadResult::Error(format!("重读 mcp.json 失败：{e}")),
        },
        ThreadCmd::ConfigRequirementsRead => {
            // 诚实清单：本进程实际会读的配置/密钥，不编造 Codex 专有项
            ThreadResult::Value(serde_json::json!({
                "requirements": [
                    {
                        "key": "DEEPSEEK_API_KEY",
                        "required": false,
                        "scope": "env",
                        "reason": "默认模型提供商；未设时若 providers.json 已配可其它服务商",
                    },
                    {
                        "key": "NEO_HOME",
                        "required": false,
                        "scope": "env",
                        "reason": "配置根目录，默认 ~/.neo",
                    },
                    {
                        "key": "$NEO_HOME/mcp.json",
                        "required": false,
                        "scope": "file",
                        "reason": "MCP 服务器清单（仅用户级，仓库级显式拒绝）",
                    },
                    {
                        "key": "$NEO_HOME/providers.json",
                        "required": false,
                        "scope": "file",
                        "reason": "模型服务商注册表",
                    },
                ],
                "configFile": config_json_path_display(),
            }))
        }
        ThreadCmd::SkillsConfigWrite { enabled, name, path } => {
            match neo_skill_loader::write_skill_config(enabled, name.as_deref(), path.as_deref()) {
                Ok(v) => ThreadResult::Value(v),
                Err(e) => ThreadResult::Error(e),
            }
        }
        ThreadCmd::SkillsExtraRootsSet { extra_roots } => {
            match neo_skill_loader::set_extra_roots(extra_roots) {
                Ok(roots) => ThreadResult::Value(serde_json::json!({ "extraRoots": roots })),
                Err(e) => ThreadResult::Error(e),
            }
        }
        ThreadCmd::ExperimentalList { cursor: _, limit, thread_id: _ } => {
            let flags = experimental_load();
            let mut items: Vec<serde_json::Value> = flags
                .as_object()
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| {
                            serde_json::json!({
                                "name": k,
                                "enabled": v.as_bool().unwrap_or(false),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            items.sort_by_key(|i| i["name"].as_str().unwrap_or("").to_string());
            if let Some(n) = limit {
                items.truncate(n as usize);
            }
            ThreadResult::Value(serde_json::json!({
                "features": items,
                "nextCursor": serde_json::Value::Null,
            }))
        }
        ThreadCmd::ExperimentalSet { enablement } => {
            let Some(map) = enablement.as_object() else {
                return ThreadResult::Error("enablement 必须是 object（feature → bool）".into());
            };
            let mut flags = experimental_load();
            if flags.as_object().is_none() {
                flags = serde_json::json!({});
            }
            for (k, v) in map {
                if let Some(b) = v.as_bool() {
                    flags[k] = serde_json::Value::Bool(b);
                }
            }
            match experimental_save(&flags) {
                Ok(()) => ThreadResult::Value(serde_json::json!({ "enablement": flags })),
                Err(e) => ThreadResult::Error(e),
            }
        }
        // 本仓无 connector/app 运行时 —— 空表诚实返回，不编造条目
        ThreadCmd::AppList { cursor: _, force_refetch: _, limit: _, thread_id: _ } => {
            ThreadResult::Value(serde_json::json!({
                "apps": [],
                "nextCursor": serde_json::Value::Null,
                "note": "NEO 尚无 apps/connectors 运行时",
            }))
        }
        ThreadCmd::AppRead { app_ids, include_tools: _, thread_id: _ } => {
            ThreadResult::Value(serde_json::json!({
                "apps": [],
                "requested": app_ids,
                "note": "NEO 尚无 apps/connectors 运行时",
            }))
        }
        ThreadCmd::AppInstalled { force_refresh: _, thread_id: _ } => {
            ThreadResult::Value(serde_json::json!({
                "apps": [],
                "note": "NEO 尚无 apps/connectors 运行时",
            }))
        }
        ThreadCmd::FuzzyFileSearch { query, roots, cancellation_token: _ } => {
            let hits = fuzzy_search_roots(&query, &roots);
            ThreadResult::Value(serde_json::json!({
                "matches": hits.matches,
                "truncated": hits.truncated,
            }))
        }
        ThreadCmd::WindowsSandboxReadiness => {
            let is_windows = cfg!(windows);
            ThreadResult::Value(serde_json::json!({
                "supported": is_windows,
                "ready": false,
                "platform": std::env::consts::OS,
                "note": if is_windows {
                    "Windows 沙箱就绪检测尚未接入"
                } else {
                    "非 Windows 平台无 Windows 沙箱"
                },
            }))
        }
        ThreadCmd::WindowsSandboxSetupStart { mode, cwd: _ } => {
            if cfg!(windows) {
                ThreadResult::Error(format!(
                    "windowsSandbox/setupStart 尚未在 Windows 上接入（mode={mode}）"
                ))
            } else {
                ThreadResult::Error(format!(
                    "windowsSandbox/setupStart 仅 Windows 适用（当前 {}）",
                    std::env::consts::OS
                ))
            }
        }
        ThreadCmd::GuardianDenied { id, event: _ } => {
            // Codex Guardian 是其专有审批评估流；NEO 无对等物 —— 明确拒绝而非静默成功
            ThreadResult::Error(format!(
                "NEO 无 Guardian 审批评估系统，无法处理 thread/approveGuardianDeniedAction（threadId={id}）"
            ))
        }
        ThreadCmd::ReviewStart { target, thread_id, delivery } => {
            // 对齐形状：在指定线程上启动一轮「代码审查」用户输入（走既有 UserTurn）
            if thread_id != kernel.session_id() && !store.exists(&thread_id) {
                return ThreadResult::Error(format!("会话 {thread_id} 不存在"));
            }
            let target_kind = target
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let prompt = match target_kind {
                "uncommittedChanges" => {
                    "请审查当前工作区未提交的改动（staged/unstaged/untracked），\
                     按严重程度列出问题，并给出可执行的修复建议。"
                        .to_string()
                }
                "baseBranch" => {
                    let base = target
                        .get("branch")
                        .and_then(|v| v.as_str())
                        .unwrap_or("main");
                    format!(
                        "请审查当前分支相对 `{base}` 的 diff，按严重程度列出问题，并给出修复建议。"
                    )
                }
                other => format!("请审查目标 `{other}` 的相关改动，列出问题与修复建议。"),
            };
            if delivery.as_deref() == Some("detached") {
                // Codex 已弃用 detached；我们不另开线，如实说明并仍 inline
                ThreadResult::Value(serde_json::json!({
                    "started": true,
                    "delivery": "inline",
                    "deprecationNotice": "detached delivery 已弃用；已在目标线程 inline 启动",
                    "threadId": thread_id,
                    "prompt": prompt,
                    "drives": false,
                    "note": "仅登记审查意图；请用 turn/start 发送 prompt 驱动模型",
                }))
            } else {
                ThreadResult::Value(serde_json::json!({
                    "started": true,
                    "delivery": "inline",
                    "threadId": thread_id,
                    "prompt": prompt,
                    "drives": false,
                    "note": "审查 prompt 已就绪；调用方用 turn/start 提交以驱动模型",
                }))
            }
        }
        ThreadCmd::FeedbackUpload {
            classification,
            reason,
            tags,
            include_logs,
            extra_log_files,
            thread_id,
        } => {
            // 无账号远端 —— 落本地收据，不假装上传成功
            let receipt_dir = neo_skill_loader::marketplace::neo_home()
                .map(|h| h.join("feedback"))
                .ok_or_else(|| "NEO_HOME/HOME 不可用".to_string());
            match receipt_dir {
                Err(e) => ThreadResult::Error(e),
                Ok(dir) => {
                    if let Err(e) = std::fs::create_dir_all(&dir) {
                        return ThreadResult::Error(e.to_string());
                    }
                    let ts = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    let path = dir.join(format!("fb-{ts}.json"));
                    let body = serde_json::json!({
                        "classification": classification,
                        "reason": reason,
                        "tags": tags,
                        "includeLogs": include_logs.unwrap_or(false),
                        "extraLogFiles": extra_log_files,
                        "threadId": thread_id,
                    });
                    if let Err(e) = std::fs::write(
                        &path,
                        serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".into()) + "\n",
                    ) {
                        return ThreadResult::Error(e.to_string());
                    }
                    ThreadResult::Value(serde_json::json!({
                        "uploaded": false,
                        "localPath": path.display().to_string(),
                        "note": "无账号远端；反馈已本地保存",
                    }))
                }
            }
        }
        // ── 批6：fs/watch · externalAgentConfig · mcp oauth ─────────────
        ThreadCmd::FsWatch { path, watch_id } => {
            let p = std::path::PathBuf::from(&path);
            let abs = if p.is_absolute() {
                p
            } else {
                kernel.cwd().join(&p)
            };
            let abs = match abs.canonicalize() {
                Ok(c) => c,
                Err(e) => {
                    return ThreadResult::Error(format!(
                        "fs/watch 路径不可用：{}（{e}）",
                        abs.display()
                    ))
                }
            };
            if !abs.is_dir() && !abs.is_file() {
                return ThreadResult::Error(format!("fs/watch 路径不存在：{}", abs.display()));
            }
            // 文件则监听其父目录（notify 对单文件 watch 行为因平台而异）
            let watch_root = if abs.is_file() {
                abs.parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| abs.clone())
            } else {
                abs.clone()
            };
            let id = watch_id.clone();
            let bus = watch_state.bus.clone();
            let sink = bus.clone();
            // 覆盖同 id：先丢旧监听
            watch_state.active.remove(&watch_id);
            match neo_platform::file_watch::PathWatcher::watch(
                &watch_root,
                Box::new(move |paths, coalesced| {
                    let strs: Vec<String> = paths
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect();
                    if let Some(b) = sink
                        .lock()
                        .ok()
                        .and_then(|g| g.clone())
                    {
                        b.push(vec![EventMsg::FsChanged {
                            watch_id: id.clone(),
                            paths: strs,
                            coalesced,
                        }]);
                    }
                }),
            ) {
                Ok(w) => {
                    watch_state.active.insert(watch_id.clone(), w);
                    ThreadResult::Value(serde_json::json!({
                        "watchId": watch_id,
                        "path": watch_root.display().to_string(),
                        "watching": true,
                    }))
                }
                Err(e) => ThreadResult::Error(format!("fs/watch 启动失败：{e}")),
            }
        }
        ThreadCmd::FsUnwatch { watch_id } => {
            let removed = watch_state.active.remove(&watch_id).is_some();
            ThreadResult::Value(serde_json::json!({
                "watchId": watch_id,
                "stopped": removed,
            }))
        }
        ThreadCmd::ExtAgentDetect {
            cwds,
            include_home,
            max_session_age_days: _,
            max_sessions,
            migration_source: _,
        } => {
            let mut roots: Vec<std::path::PathBuf> = Vec::new();
            match cwds {
                Some(list) if !list.is_empty() => {
                    roots.extend(list.into_iter().map(std::path::PathBuf::from));
                }
                _ => roots.push(kernel.cwd().to_path_buf()),
            }
            if include_home.unwrap_or(false) {
                if let Some(h) = std::env::var_os("HOME") {
                    roots.push(std::path::PathBuf::from(h));
                }
            }
            let limit = max_sessions.unwrap_or(50) as usize;
            let items = ext_agent_detect(&roots, limit);
            ThreadResult::Value(serde_json::json!({
                "migrationItems": items,
                "migrationSource": "local-fs",
                "note": "仅扫描本地常见 agent 配置文件；无远端账号",
            }))
        }
        ThreadCmd::ExtAgentImport {
            migration_items,
            migration_source: _,
            provider_id,
        } => {
            let results = ext_agent_import(&migration_items, kernel.cwd());
            ThreadResult::Value(serde_json::json!({
                "imported": results["imported"],
                "skipped": results["skipped"],
                "failed": results["failed"],
                "providerId": provider_id,
                "details": results["details"],
            }))
        }
        ThreadCmd::ExtAgentImportReadHistories => {
            let path = neo_home_display().map(|h| h.join("migration-history.json"));
            let histories = path
                .and_then(|p| std::fs::read_to_string(p).ok())
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .and_then(|v| v.get("histories").cloned())
                .unwrap_or_else(|| serde_json::json!([]));
            ThreadResult::Value(serde_json::json!({ "histories": histories }))
        }
        ThreadCmd::ExtAgentImportRecordHistory {
            item_type_results,
            provider_id,
        } => {
            let Some(dir) = neo_home_display() else {
                return ThreadResult::Error("NEO_HOME/HOME 不可用".into());
            };
            if let Err(e) = std::fs::create_dir_all(&dir) {
                return ThreadResult::Error(e.to_string());
            }
            let path = dir.join("migration-history.json");
            let mut doc = std::fs::read_to_string(&path)
                .ok()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .unwrap_or_else(|| serde_json::json!({ "histories": [] }));
            if doc.get("histories").and_then(|h| h.as_array()).is_none() {
                doc["histories"] = serde_json::json!([]);
            }
            let entry = serde_json::json!({
                "providerId": provider_id,
                "itemTypeResults": item_type_results,
                "recordedAtMs": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
            });
            doc["histories"]
                .as_array_mut()
                .expect("histories array")
                .push(entry);
            // 有界：只留最近 50 条
            if let Some(arr) = doc["histories"].as_array_mut() {
                let n = arr.len();
                if n > 50 {
                    arr.drain(0..n - 50);
                }
            }
            let Ok(pretty) = serde_json::to_string_pretty(&doc) else {
                return ThreadResult::Error("migration-history 序列化失败".into());
            };
            if let Err(e) = std::fs::write(&path, pretty + "\n") {
                return ThreadResult::Error(e.to_string());
            }
            ThreadResult::Value(serde_json::json!({
                "recorded": true,
                "path": path.display().to_string(),
            }))
        }
        ThreadCmd::McpOauthLogin {
            name,
            client_registration: _,
            scopes: _,
            thread_id: _,
            timeout_secs: _,
        } => {
            // 诚实边界：NEO 无交互式浏览器 OAuth / 账号体系。
            // 静态 token 走 $NEO_HOME/mcp.json 的 headers/env。
            ThreadResult::Error(format!(
                "mcpServer/oauth/login 未接入：NEO 无浏览器 OAuth/账号体系。\
                 请在 $NEO_HOME/mcp.json 为服务器「{name}」配置静态 headers/env 后重启会话"
            ))
        }
    }
}

/// `fs/watch` 状态：watchId → 监听器（drop 即停）。
#[derive(Default)]
struct WatchState {
    active: std::collections::HashMap<String, neo_platform::file_watch::PathWatcher>,
    /// EventBus 由 serve 在启动时注入（跨线程 clone Sender）。
    bus: Arc<std::sync::Mutex<Option<neo_host_appserver::EventBus>>>,
}

// ── 批6：外部 agent 配置探测 / 导入（纯本地文件，无账号）──────────────

/// 探测常见外部 agent 资产 → Codex `ExternalAgentConfigMigrationItem[]`。
fn ext_agent_detect(roots: &[std::path::PathBuf], limit: usize) -> Vec<serde_json::Value> {
    let mut items = Vec::new();
    let mut push = |item: serde_json::Value| {
        if items.len() < limit {
            items.push(item);
        }
    };

    for root in roots {
        // AGENTS_MD
        for name in ["CLAUDE.md", ".cursorrules", "GEMINI.md", "AGENTS.md"] {
            let p = root.join(name);
            if p.is_file() {
                push(serde_json::json!({
                    "itemType": "AGENTS_MD",
                    "description": format!("{name} → 可导入为项目指令"),
                    "cwd": root.display().to_string(),
                    "details": null,
                }));
            }
        }
        // SUBAGENTS
        let agents = root.join(".claude").join("agents");
        if agents.is_dir() {
            if let Ok(rd) = std::fs::read_dir(&agents) {
                for e in rd.flatten().take(20) {
                    if e.path().extension().and_then(|x| x.to_str()) == Some("md") {
                        push(serde_json::json!({
                            "itemType": "SUBAGENTS",
                            "description": format!(
                                "子代理 {}",
                                e.file_name().to_string_lossy()
                            ),
                            "cwd": root.display().to_string(),
                            "details": null,
                        }));
                    }
                }
            }
        }
        // COMMANDS
        let cmds = root.join(".claude").join("commands");
        if cmds.is_dir() {
            if let Ok(rd) = std::fs::read_dir(&cmds) {
                for e in rd.flatten().take(20) {
                    if e.path().extension().and_then(|x| x.to_str()) == Some("md") {
                        push(serde_json::json!({
                            "itemType": "COMMANDS",
                            "description": format!(
                                "斜杠命令 {}",
                                e.file_name().to_string_lossy()
                            ),
                            "cwd": root.display().to_string(),
                            "details": null,
                        }));
                    }
                }
            }
        }
        // MCP_SERVER_CONFIG
        for rel in [
            ".cursor/mcp.json",
            ".cursor/mcp.jsonc",
            ".claude/mcp.json",
            ".config/claude/mcp.json",
        ] {
            let p = root.join(rel);
            if p.is_file() {
                push(serde_json::json!({
                    "itemType": "MCP_SERVER_CONFIG",
                    "description": format!("{rel} → 可合并进 $NEO_HOME/mcp.json"),
                    "cwd": root.display().to_string(),
                    "details": null,
                }));
            }
        }
        // SKILLS（目录存在即报，细节在 import 时扫）
        for rel in [".claude/skills", ".cursor/skills", ".agents/skills"] {
            let p = root.join(rel);
            if p.is_dir() {
                push(serde_json::json!({
                    "itemType": "SKILLS",
                    "description": format!("{rel} → 可导入技能根"),
                    "cwd": root.display().to_string(),
                    "details": null,
                }));
            }
        }
    }
    items
}

/// 导入选中的迁移项（本地复制/合并）。部分类型如实 `skipped`。
fn ext_agent_import(
    items: &[serde_json::Value],
    cwd: &std::path::Path,
) -> serde_json::Value {
    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut details = Vec::new();

    for item in items {
        let kind = item
            .get("itemType")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let item_cwd = item
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| cwd.to_path_buf());
        let mut note = String::new();
        let mut ok = false;

        match kind {
            "AGENTS_MD" => {
                // 扫常见指令文件，拷到工作区 AGENTS.md（不存在才写，不覆盖）
                let dest = item_cwd.join("AGENTS.md");
                if dest.exists() {
                    note = "AGENTS.md 已存在，跳过覆盖".into();
                    skipped += 1;
                } else {
                    let mut wrote = false;
                    for name in ["CLAUDE.md", ".cursorrules", "GEMINI.md"] {
                        let src = item_cwd.join(name);
                        if src.is_file() {
                            match std::fs::copy(&src, &dest) {
                                Ok(_) => {
                                    note = format!("{name} → AGENTS.md");
                                    imported += 1;
                                    ok = true;
                                    wrote = true;
                                    break;
                                }
                                Err(e) => {
                                    note = format!("复制 {name} 失败：{e}");
                                    failed += 1;
                                    wrote = true;
                                    break;
                                }
                            }
                        }
                    }
                    if !wrote {
                        // 也可能是源 AGENTS.md 本身
                        note = "未找到可复制的指令源".into();
                        skipped += 1;
                    }
                }
            }
            "SKILLS" => {
                // 把 skills 根下一级 SKILL.md 目录拷进 $NEO_HOME/skills/
                let Some(home) = neo_skill_loader::marketplace::neo_home() else {
                    note = "NEO_HOME 不可用".into();
                    failed += 1;
                    details.push(serde_json::json!({"itemType": kind, "ok": false, "note": note}));
                    continue;
                };
                let dest_root = home.join("skills");
                if let Err(e) = std::fs::create_dir_all(&dest_root) {
                    note = e.to_string();
                    failed += 1;
                } else {
                    // 尝试多个候选源
                    let mut copied = 0usize;
                    for rel in [".claude/skills", ".cursor/skills", ".agents/skills"] {
                        let src_root = item_cwd.join(rel);
                        if !src_root.is_dir() {
                            continue;
                        }
                        if let Ok(rd) = std::fs::read_dir(&src_root) {
                            for e in rd.flatten() {
                                if !e.path().is_dir() {
                                    continue;
                                }
                                let name = e.file_name();
                                let dest = dest_root.join(&name);
                                if dest.exists() {
                                    continue;
                                }
                                if copy_dir_simple(&e.path(), &dest).is_ok() {
                                    copied += 1;
                                }
                            }
                        }
                    }
                    if copied > 0 {
                        note = format!("导入 {copied} 个技能目录");
                        imported += 1;
                        ok = true;
                    } else {
                        note = "无新技能可导入".into();
                        skipped += 1;
                    }
                }
            }
            "MCP_SERVER_CONFIG" => {
                // 合并 servers 到 $NEO_HOME/mcp.json（同名跳过）
                let Some(home) = neo_skill_loader::marketplace::neo_home() else {
                    note = "NEO_HOME 不可用".into();
                    failed += 1;
                    details.push(serde_json::json!({"itemType": kind, "ok": false, "note": note}));
                    continue;
                };
                let dest = home.join("mcp.json");
                // 源文件：优先 details 无路径时用 cwd 下常见路径
                let mut src_path = None;
                for rel in [".cursor/mcp.json", ".cursor/mcp.jsonc", ".claude/mcp.json"] {
                    let p = item_cwd.join(rel);
                    if p.is_file() {
                        src_path = Some(p);
                        break;
                    }
                }
                // 也接受 description 里的相对路径提示：从 item 自身 cwd 扫
                let Some(src) = src_path else {
                    note = "找不到 MCP 配置源文件".into();
                    skipped += 1;
                    details.push(serde_json::json!({"itemType": kind, "ok": false, "note": note}));
                    continue;
                };
                let Ok(src_raw) = std::fs::read_to_string(&src) else {
                    note = "读源失败".into();
                    failed += 1;
                    details.push(serde_json::json!({"itemType": kind, "ok": false, "note": note}));
                    continue;
                };
                // jsonc 容错：去 // 行注释（粗处理）
                let stripped: String = src_raw
                    .lines()
                    .filter(|l| !l.trim_start().starts_with("//"))
                    .collect::<Vec<_>>()
                    .join("\n");
                let Ok(src_val) = serde_json::from_str::<serde_json::Value>(&stripped) else {
                    note = "源不是合法 JSON".into();
                    failed += 1;
                    details.push(serde_json::json!({"itemType": kind, "ok": false, "note": note}));
                    continue;
                };
                let mut dest_val = std::fs::read_to_string(&dest)
                    .ok()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                    .unwrap_or_else(|| serde_json::json!({ "servers": [] }));
                if dest_val.get("servers").and_then(|s| s.as_array()).is_none() {
                    dest_val["servers"] = serde_json::json!([]);
                }
                let src_servers = src_val
                    .get("servers")
                    .and_then(|s| s.as_array())
                    .cloned()
                    .unwrap_or_default();
                let mut added = 0usize;
                for s in src_servers {
                    let name = s.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    if name.is_empty() {
                        continue;
                    }
                    let exists = dest_val["servers"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .any(|x| x.get("name").and_then(|n| n.as_str()) == Some(name))
                        })
                        .unwrap_or(false);
                    if !exists {
                        dest_val["servers"].as_array_mut().expect("servers").push(s);
                        added += 1;
                    }
                }
                if added > 0 {
                    if let Some(d) = dest.parent() {
                        let _ = std::fs::create_dir_all(d);
                    }
                    match std::fs::write(
                        &dest,
                        serde_json::to_string_pretty(&dest_val).unwrap_or_default() + "\n",
                    ) {
                        Ok(()) => {
                            note = format!("合并 {added} 个 MCP 服务器");
                            imported += 1;
                            ok = true;
                        }
                        Err(e) => {
                            note = e.to_string();
                            failed += 1;
                        }
                    }
                } else {
                    note = "无新 MCP 服务器".into();
                    skipped += 1;
                }
            }
            other => {
                note = format!("类型 {other} 本轮不支持导入（如实跳过）");
                skipped += 1;
            }
        }
        details.push(serde_json::json!({
            "itemType": kind,
            "ok": ok,
            "note": note,
        }));
    }

    serde_json::json!({
        "imported": imported,
        "skipped": skipped,
        "failed": failed,
        "details": details,
    })
}

fn copy_dir_simple(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let src = e.path();
        let dst = to.join(e.file_name());
        if src.is_dir() {
            copy_dir_simple(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

// ── 批5 辅助：配置 JSON / fuzzy 搜索（不依赖 host-tui，避免 L5 横向依赖）──

fn neo_home_display() -> Option<std::path::PathBuf> {
    let h = std::env::var_os("NEO_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(std::path::PathBuf::from))?;
    Some(if h.ends_with(".neo") { h } else { h.join(".neo") })
}

fn config_json_path() -> Option<std::path::PathBuf> {
    neo_home_display().map(|h| h.join("config.json"))
}

fn config_json_path_display() -> String {
    config_json_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "不可用".into())
}

fn config_load() -> serde_json::Value {
    let Some(p) = config_json_path() else {
        return serde_json::json!({});
    };
    let Ok(raw) = std::fs::read_to_string(p) else {
        return serde_json::json!({});
    };
    serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({}))
}

fn config_save(doc: &serde_json::Value) -> Result<(), String> {
    let p = config_json_path().ok_or("NEO_HOME/HOME 不可用")?;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(
        &p,
        serde_json::to_string_pretty(doc).map_err(|e| e.to_string())? + "\n",
    )
    .map_err(|e| e.to_string())
}

/// 点分 keyPath 写入。`upsert` = 沿路径合并对象；`replace` = 整键替换值。
fn config_set_key(
    doc: &mut serde_json::Value,
    key_path: &str,
    value: serde_json::Value,
    merge: &str,
) -> Result<(), String> {
    if key_path.trim().is_empty() {
        return Err("keyPath 不能为空".into());
    }
    if doc.as_object().is_none() && !doc.is_null() {
        return Err("config 根必须是 object".into());
    }
    if doc.is_null() {
        *doc = serde_json::json!({});
    }
    let parts: Vec<&str> = key_path
        .split('.')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return Err("keyPath 不能为空".into());
    }
    let mut cur = doc;
    for (i, part) in parts.iter().enumerate() {
        if cur.as_object().is_none() {
            *cur = serde_json::json!({});
        }
        let last = i + 1 == parts.len();
        if last {
            if merge == "upsert" {
                if cur.get(*part).map(|v| v.is_object()).unwrap_or(false)
                    && value.is_object()
                {
                    let obj = cur[*part].as_object_mut().expect("object");
                    for (k, v) in value.as_object().expect("object") {
                        obj.insert(k.clone(), v.clone());
                    }
                } else {
                    cur[*part] = value.clone();
                }
            } else {
                // replace（及未知策略按 replace：整值覆盖）
                cur[*part] = value.clone();
            }
        } else {
            let next = cur
                .as_object_mut()
                .expect("object")
                .entry((*part).to_string())
                .or_insert_with(|| serde_json::json!({}));
            if next.is_null() {
                *next = serde_json::json!({});
            }
            cur = next;
        }
    }
    Ok(())
}

fn config_write_single(
    key_path: &str,
    value: &serde_json::Value,
    merge_strategy: &str,
    file_path: Option<&str>,
    expected_version: Option<&str>,
) -> Result<serde_json::Value, String> {
    if let Some(fp) = file_path {
        if !fp.ends_with(".json") {
            return Err(format!(
                "本轮只写 $NEO_HOME/config.json；不支持 filePath={fp}（无 toml 装载器）"
            ));
        }
    }
    if let Some(want) = expected_version {
        let cur = config_load()
            .get("_version")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .to_string();
        if cur != want {
            return Err(format!("expectedVersion 不匹配：当前 {cur}，期望 {want}"));
        }
    }
    let mut doc = config_load();
    if doc.is_null() {
        doc = serde_json::json!({});
    }
    config_set_key(&mut doc, key_path, value.clone(), merge_strategy)?;
    // 乐观并发：每次写递增 _version
    let ver = doc
        .get("_version")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        + 1;
    doc["_version"] = serde_json::json!(ver);
    config_save(&doc)?;
    Ok(serde_json::json!({
        "keyPath": key_path,
        "value": value,
        "mergeStrategy": merge_strategy,
        "filePath": config_json_path_display(),
        "version": ver.to_string(),
        "applied": true,
    }))
}

fn config_write_batch(
    edits: &[serde_json::Value],
    file_path: Option<&str>,
    expected_version: Option<&str>,
) -> Result<serde_json::Value, String> {
    if let Some(fp) = file_path {
        if !fp.ends_with(".json") {
            return Err(format!(
                "本轮只写 $NEO_HOME/config.json；不支持 filePath={fp}"
            ));
        }
    }
    if let Some(want) = expected_version {
        let cur = config_load()
            .get("_version")
            .and_then(|v| v.as_str())
            .unwrap_or("0")
            .to_string();
        if cur != want {
            return Err(format!("expectedVersion 不匹配：当前 {cur}，期望 {want}"));
        }
    }
    let mut doc = config_load();
    if doc.is_null() {
        doc = serde_json::json!({});
    }
    let mut applied = 0usize;
    for (i, e) in edits.iter().enumerate() {
        let key = e
            .get("keyPath")
            .or_else(|| e.get("key_path"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("edits[{i}].keyPath 缺失"))?;
        let merge = e
            .get("mergeStrategy")
            .or_else(|| e.get("merge_strategy"))
            .and_then(|v| v.as_str())
            .unwrap_or("replace");
        let val = e.get("value").cloned().unwrap_or(serde_json::Value::Null);
        config_set_key(&mut doc, key, val, merge)?;
        applied += 1;
    }
    let ver = doc
        .get("_version")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        + 1;
    doc["_version"] = serde_json::json!(ver);
    config_save(&doc)?;
    Ok(serde_json::json!({
        "applied": applied,
        "filePath": config_json_path_display(),
        "version": ver.to_string(),
    }))
}

fn experimental_path() -> Option<std::path::PathBuf> {
    neo_home_display().map(|h| h.join("experimental.json"))
}

fn experimental_load() -> serde_json::Value {
    let Some(p) = experimental_path() else {
        return serde_json::json!({});
    };
    let Ok(raw) = std::fs::read_to_string(p) else {
        return serde_json::json!({});
    };
    serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({}))
}

fn experimental_save(flags: &serde_json::Value) -> Result<(), String> {
    let p = experimental_path().ok_or("NEO_HOME/HOME 不可用")?;
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::write(
        &p,
        serde_json::to_string_pretty(flags).map_err(|e| e.to_string())? + "\n",
    )
    .map_err(|e| e.to_string())
}

/// 子序列模糊匹配 + 连续/边界加权（与 TUI `fuzzy_score` 同启发，不依赖 host-tui）。
fn fuzzy_score(query: &str, candidate: &str) -> Option<u32> {
    if query.is_empty() {
        return Some(0);
    }
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let c: Vec<char> = candidate.to_lowercase().chars().collect();
    let orig: Vec<char> = candidate.chars().collect();
    let mut qi = 0usize;
    let mut score = 0u32;
    let mut last: Option<usize> = None;
    for (ci, &ch) in c.iter().enumerate() {
        if qi >= q.len() {
            break;
        }
        if ch == q[qi] {
            if let Some(lm) = last {
                if ci == lm + 1 {
                    score += 1;
                } else {
                    score += 3 + (ci - lm) as u32;
                }
            } else {
                score += ci as u32;
                let boundary = ci == 0
                    || matches!(
                        orig.get(ci.wrapping_sub(1)),
                        Some('/') | Some('_') | Some('-') | Some('.')
                    );
                if boundary {
                    score = score.saturating_sub(2);
                }
            }
            last = Some(ci);
            qi += 1;
        }
    }
    if qi == q.len() { Some(score) } else { None }
}

struct FuzzyHits {
    matches: Vec<serde_json::Value>,
    truncated: bool,
}

fn fuzzy_search_roots(query: &str, roots: &[String]) -> FuzzyHits {
    const SKIP: &[&str] = &[
        ".git", "node_modules", "target", "dist", "build", ".venv", "__pycache__", ".next",
        ".cache", ".neo",
    ];
    const MAX_FILES: usize = 20_000;
    const MAX_DEPTH: usize = 8;
    const MAX_HITS: usize = 50;
    let mut scored: Vec<(u32, String)> = Vec::new();
    let mut truncated = false;
    let mut scanned = 0usize;
    for root in roots {
        let rootp = std::path::Path::new(root);
        if !rootp.is_dir() {
            continue;
        }
        let mut stack = vec![(rootp.to_path_buf(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            if depth > MAX_DEPTH || scanned >= MAX_FILES {
                truncated = true;
                continue;
            }
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                if scanned >= MAX_FILES {
                    truncated = true;
                    break;
                }
                scanned += 1;
                let path = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                if path.is_dir() {
                    if SKIP.contains(&name.as_str()) {
                        continue;
                    }
                    stack.push((path, depth + 1));
                } else if let Some(s) = fuzzy_score(query, &name) {
                    // 相对 root 的展示路径优先
                    let display = path
                        .strip_prefix(rootp)
                        .map(|r| r.display().to_string())
                        .unwrap_or_else(|_| path.display().to_string());
                    scored.push((s, display));
                }
            }
        }
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    scored.dedup_by(|a, b| a.1 == b.1);
    let truncated = truncated || scored.len() > MAX_HITS;
    let matches: Vec<serde_json::Value> = scored
        .into_iter()
        .take(MAX_HITS)
        .map(|(score, path)| serde_json::json!({ "path": path, "score": score }))
        .collect();
    FuzzyHits { matches, truncated }
}

/// 启动桌面窗口（三种实现：GPUI 默认 / --egui / --webview）。
///
/// 窗口只是壳：完整复用 Web 宿主（本地回环端口 + 内置页面），
/// T6 宿主等价因此天然成立 —— 桌面跑的就是 Web 宿主，没有第三套
/// 事件消费逻辑要证明等价。窗口关闭 = 退出应用（随 op 通道关闭，
/// 内核线程停机）。
///
/// 仅在 `desktop` feature 开启时编译（默认开启）；精简构建下
/// `neo desktop` 由 `main` 里的占位分支给出明确提示，而非静默不存在。
#[cfg(feature = "desktop")]
fn cmd_desktop(args: &[String]) -> i32 {
    let mut workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    // 默认空 = 自动选择：设置页配置的注册表服务商优先，其次 env deepseek
    // （选择逻辑见 build_models）。硬编码 "deepseek" 会让只配了智谱的用户
    // 直接 `neo` 时被要求 DEEPSEEK_API_KEY（真实反馈）。
    let mut provider = String::new();
    let mut mode = ExecMode::Default;
    // 桌面宿主的三种窗口实现：
    //   默认      → GPUI（NEO 的桌面 UI）
    //   --egui    → 旧原生 GUI（已冻结，保留作回退通道）
    //   --webview → 系统 webview（复用 Web 宿主，ADR-0006 保留的第二后端）
    let mut use_webview = false;
    let mut use_egui = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--webview" => use_webview = true,
            // `--gpui` 保留为**无操作**而不是删掉：脚本/文档里用过它。
            // 删掉会让 `neo desktop --gpui` 报"未知参数"—— 而它现在正是默认，
            // 报错会让人以为这个选项坏了。
            "--gpui" => {}
            "--egui" => use_egui = true,
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
                eprintln!("[neo] desktop 不认识的参数：{other}");
                return 2;
            }
        }
        i += 1;
    }

    let Some(models) = build_models(&provider) else {
        return 2;
    };
    // 横幅在内核装配（models 被移走）之后才打印，先取下实际选中的名字
    let model_name = models.current_provider().name().to_string();
    let sandbox = Arc::new(neo_sandbox_local::LocalSandbox::new(&workspace));
    let persistence = Box::new(neo_session_local::JsonlPersistence::new(
        workspace.join(".neo/sessions/neo-desktop.jsonl"),
    ));
    let opts = ExecOptions { mode, max_steps: 32, ..Default::default() };
    let kernel = build_kernel("neo-desktop", &workspace, &opts, models, sandbox, persistence);

    eprintln!("[neo] 工作区 {}", workspace.display());
    eprintln!("[neo] 模式   {}", describe_mode(mode));
    eprintln!("[neo] 模型   {model_name}");

    // ── 原生 GUI（默认）────────────────────────────────────────────
    //
    // 不经 HTTP、不开端口：内核独占驱动线程，UI 直接消费 `EventMsg`。
    // 与 webview 版相比少了整条 HTTP/SSE 链路 —— 那条链路存在的原因是
    // "窗口是个浏览器"，原生宿主不需要它（也顺带没有了"本地端口谁能访问"
    // 这个问题）。
    // ── GPUI 宿主（`--gpui`）──────────────────────────────────────
    //
    // 放在 egui 分支**之前**：显式指定优先。两条路的装配完全一样
    // （同一个 build_kernel / 同一份 sessions / 同一个驱动），差别只在谁渲染。
    #[cfg(feature = "gpui")]
    if !use_webview && !use_egui {
        // 模型清单：装配期取一次（kernel 随后被 move 进驱动线程）。
        //
        // ⚠️ 取**完整信息**（说明 + 是否桩），不只取名字：宿主的选择器要标出
        // 桩 provider（mock/selftest 跑不了真实任务），还要能显示说明。
        // 只传名字的后果是用户切到桩之后以为模型坏了。
        let models: Vec<neo_driver::transcript::ModelChoice> = kernel
            .available_models()
            .into_iter()
            .map(|m| {
                neo_driver::transcript::ModelChoice::new(m.name, m.description, m.production)
            })
            .collect();

        let (handle, cmd_rx, batch_tx) = neo_driver::channel();
        // 响应式宿主：事件到达时要主动唤醒重绘（gpui 不出帧就不画）。
        // 信号式（非 Send 的 gpui 上下文由宿主在自己线程消费）。
        let wake = neo_driver::WakeSignal::new();
        let kernel_thread =
            neo_driver::spawn(kernel, cmd_rx, batch_tx, Some(wake.hook()));

        let sessions_dir = workspace.join(".neo/sessions");
        let store = neo_session_store::SessionStore::open(&sessions_dir);
        let sessions: Option<Box<dyn neo_session::SessionControl>> =
            Some(Box::new(Sessions::new(handle.clone(), store)));

        eprintln!("[neo] 桌面窗口（GPUI；--egui 可切回旧实现，--webview 可切 webview）");
        let status = format!("{} · {}", workspace.display(), neo_exec::mode_short(mode));
        let result = neo_host_gpui::run(
            handle,
            sessions,
            models,
            "NEO".to_string(),
            status,
            mode,
            model_name.clone(),
            // 工作区传给宿主：文件树（D12）要扫它。由这里注入而不是宿主
            // `current_dir()` —— 宿主用的目录必须与内核用的**同一个**
            //（否则文件树指向一处、`@文件` 解析到另一处）。
            workspace.clone(),
            wake,
        );
        let _ = kernel_thread.join();
        if let Err(e) = result {
            eprintln!("[neo] {e}");
            return 1;
        }
        return 0;
    }
    #[cfg(not(feature = "gpui"))]
    if !use_webview && !use_egui {
        eprintln!("[neo] 本二进制未编译 GPUI 宿主（构建时缺 feature `gpui`）。");
        eprintln!("      它现在是默认桌面窗口。可用 --egui 走旧实现，");
        eprintln!("      或重装并保留该 feature：cargo install neo-code-cli --features gpui");
        return 2;
    }

    #[cfg(feature = "egui")]
    if !use_webview && use_egui {
        // 模型列表要在**装配期**取：`kernel` 一旦 move 进驱动线程，UI 就只剩
        // 事件流可用（而"有哪些模型可选"不是事件，是启动时的已知状态）。
        // 切换模型仍然走 Op::ConfigureSession 提交给内核（宿主不持有内核）。
        let models = kernel
            .available_models()
            .into_iter()
            .map(|m| (m.name, m.description, m.production))
            .collect::<Vec<_>>();

        let (handle, cmd_rx, batch_tx) = neo_host_egui::driver::channel();
        // egui 是即时模式：每帧自己 drain，不需要唤醒钩子
        let kernel_thread = neo_host_egui::driver::spawn(kernel, cmd_rx, batch_tx, None);

        // D1：会话栏需要会话库。用 `Sessions<H>`（泛型在句柄类型上，
        // 与 TUI 共用同一份会话逻辑 —— 见 `KernelAccess` 的说明）。
        let sessions_dir = workspace.join(".neo/sessions");
        let store = neo_session_store::SessionStore::open(&sessions_dir);
        let sessions: Option<Box<dyn neo_session::SessionControl>> =
            Some(Box::new(Sessions::new(handle.clone(), store)));

        eprintln!("[neo] 桌面窗口（egui 旧实现；它已冻结，只修 bug）");
        let status = format!(
            "{} · {}",
            workspace.display(),
            // 模式与模型不再塞进状态串：它们在状态行里有各自的可用控件
            // （可点击切换），重复显示会占地方也说不出更多信息
            neo_exec::mode_short(mode)
        );
        let result =
            neo_host_egui::ui::run(handle, "NEO", status, mode, model_name.clone(), models, sessions);
        // 窗口已关：驱动线程的通道随之关闭，内核线程停机
        let _ = kernel_thread.join();
        if let Err(e) = result {
            eprintln!("[neo] {e}");
            return 1;
        }
        return 0;
    }

    #[cfg(not(feature = "egui"))]
    if !use_webview && use_egui {
        eprintln!("[neo] 本二进制未编译 egui 宿主（构建时缺 feature `egui`）。");
        eprintln!("      默认桌面窗口是 GPUI，不需要它；要回退旧实现才需要：");
        eprintln!("      cargo install neo-code-cli --features egui");
        return 2;
    }

    // ── webview（--webview）────────────────────────────────────────
    //
    // 只绑回环 + 临时端口。绑回环**不等于**访问控制：本机任意进程都能枚举
    // 端口，浏览器里的任意网页也能跨源 POST（端点是简单请求，请求会生效）。
    // 因此窗口加载的是带访问令牌的 page_url —— 恶意网页拿不到令牌
    // （它在另一个源的 URL 里，跨源 fetch 读不到）。
    let mut kernel = kernel;
    let (server, kernel_thread) = match neo_host_web::start("127.0.0.1:0", move |op| {
        kernel
            .submit(op)
            .unwrap_or_else(|e| vec![EventMsg::Error { message: e.to_string() }])
    }) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[neo] 无法启动本地服务：{e}");
            return 2;
        }
    };
    let url = server.page_url();
    eprintln!("[neo] 桌面窗口（webview）{url}（关闭窗口即退出）");

    // 事件流不重放：窗口先连上 SSE 再提交任务 —— 页面加载即建连，
    // 与 serve 同一约定。
    let served = server; // 持有 server：ops 通道在窗口关闭前必须活着
    let result = neo_host_desktop::window::run_window(&url, "NEO");
    drop(served); // 窗口已关：关闭 op 通道，内核线程停机
    let _ = kernel_thread.join();
    if let Err(e) = result {
        eprintln!("[neo] {e}");
        return 1;
    }
    0
}

fn cmd_exec(args: &[String]) -> i32 {
    let mut opts = ExecOptions::default();
    let mut workspace = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    // 默认空 = 自动选择：设置页配置的注册表服务商优先，其次 env deepseek
    // （选择逻辑见 build_models）。硬编码 "deepseek" 会让只配了智谱的用户
    // 直接 `neo` 时被要求 DEEPSEEK_API_KEY（真实反馈）。
    let mut provider = String::new();
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
            "--goal" => {
                i += 1;
                match args.get(i) {
                    Some(g) if !g.trim().is_empty() => opts.goal = Some(g.clone()),
                    _ => {
                        eprintln!("[neo] --goal 需要目标文本（多行目标用 $'...' 传入）");
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

    // 目标与任务互斥：混用会让"到底执行哪个"变成谜语
    if opts.goal.is_some() && !task_parts.is_empty() {
        eprintln!("[neo] --goal 与任务描述互斥：目标模式用 --goal，普通任务直接给描述");
        return 2;
    }
    if opts.goal.is_none() {
        if task_parts.is_empty() {
            eprintln!("[neo] 缺少任务描述。示例：neo exec \"列出当前目录的文件\"（或用 --goal 进入目标模式）");
            return 2;
        }
        opts.task = task_parts.join(" ");
    }

    // ── 装配：provider / sandbox / persistence ──────────────────────────
    let Some(models) = build_models(&provider) else {
        return 2;
    };
    // 横幅在内核装配（models 被移走）之后才打印，先取下实际选中的名字
    let model_name = models.current_provider().name().to_string();

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
        eprintln!("[neo] 模型   {model_name}\n");
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
    // 默认空 = 自动选择：设置页配置的注册表服务商优先，其次 env deepseek
    // （选择逻辑见 build_models）。硬编码 "deepseek" 会让只配了智谱的用户
    // 直接 `neo` 时被要求 DEEPSEEK_API_KEY（真实反馈）。
    let mut provider = String::new();
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
        // 上下文窗口取**当前模型注册的真实值**（用户配置的 providers.json
        // 条目带 context_limit）。写死会把智谱 128k 标成 64k，侧栏占用条
        // 直接翻倍失真。取不到（0）时侧栏显示"上限未知"，宁可不给假数字。
        context_limit: kernel
            .available_models()
            .into_iter()
            .find(|m| m.name == kernel.current_model())
            .map(|m| m.context_limit)
            .unwrap_or(0),
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

    // 内核挪到**驱动线程**（独占）：TUI 的 submit(Pump) 变成
    // "发出 op + 最多等一个时间片收事件批"，工具执行 / 首字延迟期间
    // 主线程照常按时间片重绘（spinner 转）—— 不再冻结（真实反馈）。
    // Kernel: Send 由 Web 宿主（内核跑在线程里）既有代码证明。
    let (kernel_handle, cmd_rx, batch_tx) = tui_driver::channel();
    tui_driver::spawn(kernel, cmd_rx, batch_tx);

    // 会话控制：宿主调契据，CLI 经句柄在驱动线程上执行（含换内核与历史重建）。
    let mut sessions = Sessions::new(kernel_handle.clone(), store);
    let mut providers = TuiProviders::new(kernel_handle.clone());
    let result = neo_host_tui::run(about, gate, theme_name, &mut sessions, &mut providers, move |op| {
        match op {
            // Pump：最多等一个时间片，批没到返回空批 —— 泵循环照常重绘
            neo_protocol::Op::Pump => kernel_handle.pump_collect(),
            other => Ok(kernel_handle.call(other)),
        }
    });
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("[neo] TUI 启动失败：{e}");
            2
        }
    }
}


/// 「能在内核上跑一段闭包」的最小端口。
///
/// # 为什么需要它（而不是每个宿主写一份会话逻辑）
///
/// 会话切换要同时做两件事：让内核换会话并重建上下文、再把历史转回事件流给宿主。
/// 这段逻辑是**宿主无关**的。但每个宿主的内核句柄是**各自类型**（A3 禁止宿主
/// 互相依赖，驱动层各自实现）—— 于是直接复用会卡在"句柄类型不同"上。
///
/// 抽这个两行端口，两端各实现一次，会话逻辑就**只有一份**：
/// `Sessions<H>` 对两种句柄都能用。否则那 120 行（persistence_for /
/// history_of / switch / create / delete 的边界处理）要抄两遍，
/// 而抄一遍就意味着将来修一处忘一处。
trait KernelAccess {
    /// 在内核上执行闭包并取回结果（内核在驱动线程上，故需 `Send`）。
    fn with_kernel<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut neo_core::Kernel) -> R + Send + 'static,
    ) -> Option<R>;
}

impl KernelAccess for KernelHandle {
    fn with_kernel<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut neo_core::Kernel) -> R + Send + 'static,
    ) -> Option<R> {
        self.query(f)
    }
}

// 两个 GUI 宿主用的是**同一个**句柄类型（都来自 neo-driver），所以一个 impl 够。
impl KernelAccess for neo_driver::KernelHandle {
    fn with_kernel<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut neo_core::Kernel) -> R + Send + 'static,
    ) -> Option<R> {
        // GUI 版名字不同（`query_blocking`），语义一致：发出 + 等回执。
        // 它**会阻塞**，只在用户的显式动作（切会话/新建）里调用 ——
        // 不在每帧路径上，所以不违反"一帧都不等"的约束。
        self.query_blocking(f)
    }
}

/// 会话控制实现：持有内核访问端口与会话库，执行列举/切换/新建/删除。
///
/// 它活在 CLI 层而不是宿主里，因为**只有 CLI 知道内核怎么装配** ——
/// 宿主只调用契据（`SessionControl`），不碰内核类型。
/// 泛型 `H` 让它同时服务于 TUI 与桌面 GUI（见 [`KernelAccess`]）。
struct Sessions<H: KernelAccess> {
    /// 内核在驱动线程上，经句柄通信（见各宿主的 driver 模块注释）
    k: H,
    /// 会话库：会话的列举/新建/删除都在这里（日志路径由它给出）
    store: neo_session_store::SessionStore,
}

impl<H: KernelAccess> Sessions<H> {
    fn new(k: H, store: neo_session_store::SessionStore) -> Self {
        Self { k, store }
    }

    /// 为某个会话 id 造一个 JSONL 持久化（指向该会话自己的文件）。
    fn persistence_for(&self, id: &str) -> Box<dyn neo_core::SessionPersistence> {
        Box::new(neo_session_local::JsonlPersistence::new(self.store.path_for(id)))
    }

    /// 从落盘日志重建历史事件流（宿主重画转录用）。
    fn history_of(k: &mut neo_core::Kernel) -> Vec<neo_protocol::EventMsg> {
        k.log_for_test()
            .into_iter()
            .filter(|rec| rec.kind == "event")
            .filter_map(|rec| serde_json::from_value::<neo_protocol::EventMsg>(rec.payload).ok())
            .collect()
    }
}

/// 由注册表条目 + 密钥造一个 provider 与其元信息。
///
/// 抽成函数是为了让"启动时注册"与"设置页新增时热加载"走**同一段**构造逻辑 ——
/// 两处各写一遍必然会漂移（比如一边规范化 base_url、另一边忘了）。
fn provider_of(
    e: &neo_providers::ProviderEntry,
    key: &str,
) -> (neo_core::models::ModelInfo, std::sync::Arc<dyn neo_core::ModelProvider>) {
    let (host, url_path) = e
        .base_url
        .as_deref()
        .map(neo_providers::normalize_base_url)
        .unwrap_or_default();
    let p = neo_llm_deepseek::DeepSeekProvider {
        api_key: key.to_string(),
        endpoint: if host.is_empty() {
            neo_llm_deepseek::DEFAULT_ENDPOINT.to_string()
        } else {
            host
        },
        path: if url_path.is_empty() {
            neo_llm_deepseek::DEFAULT_PATH.to_string()
        } else {
            format!("{}/chat/completions", url_path.trim_end_matches('/'))
        },
        model: e.model.clone().unwrap_or_else(|| "deepseek-chat".to_string()),
        temperature: 0.0,
        label: e.name.clone(),
    };
    let info = neo_core::models::ModelInfo {
        name: e.name.clone(),
        description: e
            .description
            .clone()
            .unwrap_or_else(|| "用户级 providers.json 注册".to_string()),
        context_limit: e.context_limit,
        production: e.production,
    };
    (info, std::sync::Arc::new(p))
}

/// TUI 的服务商管理实现。
///
/// 活在 CLI 层而不是宿主里：宿主不碰配置文件；密钥的读写策略
/// （只读用户级、0600）在 `neo-providers` 里实现，这里只做转发 ——
/// 策略只有一处，宿主与 CLI 都无法绕过它。
struct TuiProviders {
    /// 内存中的注册表（磁盘是持久层，这里是在用的那份）
    registry: neo_providers::ProviderRegistry,
    /// 已存的密钥（按服务商名），与注册表分文件保存
    keys: std::collections::BTreeMap<String, String>,
    /// 内核句柄：新增/删除服务商后**立刻**把 provider 装进内核，
    /// 这样不必重启进程（"重启后生效"对正在跑的会话等于不可用）。
    /// 内核在驱动线程上，经句柄通信（见 tui_driver 模块注释）。
    k: KernelHandle,
}

impl TuiProviders {
    fn new(k: KernelHandle) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let registry = match neo_providers::load(&cwd) {
            neo_providers::LoadOutcome::Loaded(r) => r,
            // 读不动时不覆盖磁盘上的内容：以空注册表开始编辑，
            // 但**不会**在保存前把它写掉（保存是显式动作）。
            _ => neo_providers::ProviderRegistry::default(),
        };
        Self { registry, keys: neo_providers::load_keys(), k }
    }

    /// 把某个服务商装进内核（若有可用密钥）。没密钥就只是"存了配置"，
    /// 不进模型列表 —— 与启动时的行为一致（缺 key 的条目跳过并提示）。
    fn hot_load(&mut self, name: &str) -> Result<(), String> {
        let Some(entry) = self.registry.get(name).cloned() else {
            return Ok(());
        };
        let Some(key) = neo_providers::key_for(&entry, &self.keys) else {
            return Ok(()); // 只有配置、还没密钥：不算错误
        };
        let (info, p) = provider_of(&entry, &key);
        self.k
            .query(move |k| k.add_model(info, p))
            .ok_or_else(|| "内核线程已退出".to_string())?
    }

    fn save(&self) -> Result<(), String> {
        let path = neo_providers::resolve_path()
            .ok_or_else(|| "无法确定 NEO_HOME".to_string())?;
        neo_providers::save(&self.registry, &path)?;
        neo_providers::save_keys(&self.keys)
    }
}

impl neo_host_tui::ProviderControl for TuiProviders {
    fn list(&self) -> Vec<(String, String, bool)> {
        self.registry
            .providers
            .iter()
            .map(|p| {
                let has = neo_providers::key_for(p, &self.keys).is_some();
                (p.name.clone(), p.description.clone().unwrap_or_default(), has)
            })
            .collect()
    }

    fn models(&self) -> Vec<(String, String, bool)> {
        self.k
            .query(|k| {
                k.available_models()
                    .into_iter()
                    .map(|m| (m.name, m.description, m.production))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn current_model(&self) -> String {
        self.k
            .query(|k| k.current_model().to_string())
            .unwrap_or_default()
    }

    fn get(&self, name: &str) -> Option<(String, String, String, u64)> {
        self.registry.get(name).map(|p| {
            (
                p.base_url.clone().unwrap_or_default(),
                p.model.clone().unwrap_or_default(),
                p.description.clone().unwrap_or_default(),
                p.context_limit,
            )
        })
    }

    fn upsert(
        &mut self,
        name: &str,
        base_url: &str,
        model: &str,
        api_key: &str,
        description: &str,
        context_limit: u64,
        editing_original: Option<&str>,
    ) -> Result<(), String> {
        // 改名（编辑时改了 name）要先删旧条目，否则会留下一个孤儿
        if let Some(old) = editing_original {
            if old != name {
                self.registry.remove(old);
                self.keys.remove(old);
            }
        }
        let entry = neo_providers::ProviderEntry {
            name: name.to_string(),
            base_url: if base_url.is_empty() { None } else { Some(base_url.to_string()) },
            model: if model.is_empty() { None } else { Some(model.to_string()) },
            api_key_env: None,
            description: if description.is_empty() { None } else { Some(description.to_string()) },
            context_limit,
            production: true,
        };
        self.registry.upsert(entry);
        // 密钥留空 = 不改动已存的（避免让用户重新粘贴一遍）
        if !api_key.is_empty() {
            self.keys.insert(name.to_string(), api_key.to_string());
        }
        self.save()?;
        // 立刻装进内核：若该服务商有密钥，马上就能切过去用，不必重启。
        self.hot_load(name)
    }

    fn delete(&mut self, name: &str) -> Result<bool, String> {
        let removed = self.registry.remove(name);
        let had_key = self.keys.remove(name).is_some();
        if removed || had_key {
            self.save()?;
            // 从内核里摘掉（正在使用的那个会被拒绝 —— 如实报给用户）
            let name = name.to_string();
            if let Some(Err(e)) = self.k.query(move |k| k.remove_model(&name)) {
                return Err(e);
            }
        }
        Ok(removed)
    }


}

impl<H: KernelAccess> neo_session::SessionControl for Sessions<H> {
    fn list(&self) -> Vec<neo_session::SessionInfo> {
        self.store
            .list()
            .into_iter()
            .map(|m| neo_session::SessionInfo {
                id: m.id,
                title: m.title,
                records: m.records,
                changes: m.changes,
                state: m.state,
            })
            .collect()
    }

    fn switch(&mut self, id: &str) -> Result<Vec<neo_protocol::EventMsg>, String> {
        if !self.store.exists(id) {
            return Err(format!("会话 {id} 不存在"));
        }
        let p = self.persistence_for(id);
        let id = id.to_string();
        // 内核换会话并重建历史；历史**转回事件流**给宿主重画转录
        // （从落盘日志直接取 event 记录，保持与原始流一致）。
        self.k
            .with_kernel(move |k| {
                k.switch_session(id, p);
                Sessions::<H>::history_of(k)
            })
            .ok_or_else(|| "内核线程已退出".to_string())
    }

    fn create(&mut self) -> Result<String, String> {
        let id = self.store.new_id();
        let p = self.persistence_for(&id);
        let new_id = id.clone();
        self.k
            .with_kernel(move |k| k.switch_session(new_id, p))
            .ok_or_else(|| "内核线程已退出".to_string())?;
        Ok(id)
    }

    fn delete(&mut self, id: &str) -> Result<bool, String> {
        let current = self
            .k
            .with_kernel(|k| k.session_id().to_string())
            .ok_or_else(|| "内核线程已退出".to_string())?;
        if id == current {
            return Err("不能删除当前正在使用的会话（先切换到别的会话）".into());
        }
        self.store.delete(id).map_err(|e| e.to_string())
    }

    fn current(&self) -> String {
        self.k
            .with_kernel(|k| k.session_id().to_string())
            .unwrap_or_default()
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
              p: std::sync::Arc<dyn neo_core::ModelProvider>| {
        (ModelInfo { name: name.into(), description: desc.into(), context_limit: limit, production }, p)
    };

    let mut entries: Vec<(ModelInfo, std::sync::Arc<dyn neo_core::ModelProvider>)> = Vec::new();

    // 真实模型：只在 key 可用时注册。缺 key 时不给一个"假 deepseek"条目 ——
    // 那会让 /models 列出一个切过去就报错的选项。
    let mut deepseek_ok = false;
    match neo_llm_deepseek::DeepSeekProvider::from_env() {
        Ok(p) => {
            entries.push(mk("deepseek", "DeepSeek chat-completions（真实模型）", 64_000, true, std::sync::Arc::new(p)));
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
        std::sync::Arc::new(neo_llm_deepseek::ScriptedProvider::text_only(
            "（mock provider）本回答由确定性桩产生，未调用真实模型。"))));
    entries.push(mk("demo", "演示渲染：Markdown + 任务清单", 0, false,
        std::sync::Arc::new(neo_llm_deepseek::ScriptedProvider::demo().with_name("demo"))));
    entries.push(mk("selftest", "自检：按脚本调一次 apply_patch", 0, false,
        std::sync::Arc::new(neo_llm_deepseek::ScriptedProvider::scripted(
            vec![vec![neo_llm_deepseek::tool_call(
                "apply_patch",
                serde_json::json!({
                    "path": "selftest.txt",
                    "new": "由 selftest provider 经 apply_patch 写入。\n",
                }),
            )]],
            "selftest 脚本执行完毕（工具是否成功见上方工具行与失败原因）。",
        ).with_name("selftest"))));
    // 一轮内**连续多次**工具调用：D4 工具分组的唯一可离线复现的触发条件。
    //
    // 分组只在"同一轮里 ≥2 个调用"时出现（轮摘要会把两轮隔开），所以
    // 单次调用的 selftest 与只回正文的 mock 都验不到它 —— 缺了这个桩，
    // "分组到底长什么样"就只能靠读代码想象。
    entries.push(mk("multitool", "演示工具分组：一轮内连续 3 次调用", 0, false,
        std::sync::Arc::new(neo_llm_deepseek::ScriptedProvider::scripted(
            vec![vec![
                neo_llm_deepseek::tool_call_n(0, "bash", serde_json::json!({"cmd": "echo 第一次"})),
                neo_llm_deepseek::tool_call_n(1, "bash", serde_json::json!({"cmd": "echo 第二次"})),
                neo_llm_deepseek::tool_call_n(2, "bash", serde_json::json!({"cmd": "echo 第三次"})),
            ]],
            "multitool 脚本执行完毕：上面 3 次调用属于同一轮，应当折叠成一组。",
        ).with_name("multitool"))));
    // **部分修改**的演示桩：改大文件里的**一行**，于是审批预览会带上下文
    //（前后各 `CONTEXT` 行），而不是"整文件重写"那种全是增减行的形状。
    //
    // 为什么需要它：其余桩（`selftest` / `multitool`）要么整文件重写、要么
    // 只有 bash 调用，都**产不出带上下文的 diff**。而没有这个桩，
    // "审批时到底能看到多少上下文"就只能靠读代码推断 —— 本轮把上下文
    // 半径从 3 提到 10 这件事**没有一个可离线复现的观察窗口**。
    // 它同时是"折叠未改区块"唯一可离线触发的场景（需连续 >6 行上下文）。
    entries.push(mk("edit", "演示部分修改：改大文件里的一行（预览带上下文）", 0, false,
        std::sync::Arc::new(neo_llm_deepseek::ScriptedProvider::scripted(
            vec![vec![neo_llm_deepseek::tool_call(
                "apply_patch",
                serde_json::json!({
                    "path": "edit-me.txt",
                    "old": (1..=40).map(|i| format!("第 {i} 行\n")).collect::<String>(),
                    "new": (1..=40)
                        .map(|i| if i == 20 { "第 20 行（已改）\n".to_string() } else { format!("第 {i} 行\n") })
                        .collect::<String>(),
                }),
            )]],
            "partial 脚本执行完毕：只改了第 20 行，预览里应能看到前后上下文。",
        ).with_name("edit"))));
    // 报 token 的演示桩：所有其它桩都不发 Usage，于是"用量图表"离线永远
    // 画不出来（token 恒为 0）。图表必须看到形状才能判断对不对。
    entries.push(mk("usage", "演示用量趋势：多轮 token 递增（供用量图表验证）", 0, false,
        std::sync::Arc::new(neo_llm_deepseek::ScriptedProvider::usage_demo())));

    // ── 用户级注册表里的服务商（providers.json）──────────────────────
    //
    // 只在**用户级**读取；项目级文件存在会被明确拒绝（安全敏感配置，
    // 见 neo-providers 的模块说明）。读失败要如实打印原因 ——
    // 静默忽略会让用户对着"配了却不生效"想不通。
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let key_store = neo_providers::load_keys();
    let mut registry_names: Vec<String> = Vec::new();
    match neo_providers::load(&cwd) {
        neo_providers::LoadOutcome::Loaded(reg) => {
            for e in &reg.providers {
                // 密钥：**先密钥库（设置页里填的），再环境变量**。
                // 缺了跳过并提示，不给一个"切过去就连不上"的条目。
                let key = neo_providers::key_for(e, &key_store);
                let Some(key) = key else {
                    let hint = e.api_key_env.as_deref().unwrap_or("(未声明 api_key_env)");
                    eprintln!(
                        "[neo] 服务商 {} 已跳过：设置页未填密钥，环境变量 {hint} 也未设置",
                        e.name
                    );
                    continue;
                };
                // 构造逻辑与"设置页新增时热加载"共用 provider_of，
                // 避免两处漂移（比如一处规范化 base_url、另一边忘了）。
                let (info, p) = provider_of(e, &key);
                registry_names.push(info.name.clone());
                entries.push((info, p));
            }
        }
        neo_providers::LoadOutcome::Absent => {}
        neo_providers::LoadOutcome::Failed(msg) => {
            eprintln!("[neo] providers.json 未生效：{msg}");
        }
    }

    // 未指定 --provider（空串）时自动选择：**设置页配置的注册表服务商
    // 优先**（用户在设置页配置是显式意图），其次 env deepseek，
    // 都没有则给出可操作的指引。
    let provider_name: String = if provider.is_empty() {
        let pick = entries
            .iter()
            .find(|(i, _)| i.production && registry_names.contains(&i.name))
            .or_else(|| entries.iter().find(|(i, _)| i.production))
            .map(|(i, _)| i.name.clone());
        let Some(picked) = pick else {
            eprintln!("[neo] 没有可用的真实模型：先配置服务商再启动");
            eprintln!("       TUI 内 /settings → 服务商：填 base_url 与密钥（如智谱 open.bigmodel.cn/api/paas/v4）");
            eprintln!("       或 export DEEPSEEK_API_KEY=sk-... 后直接启动");
            eprintln!("       或离线试用：--provider mock | selftest");
            return None;
        };
        eprintln!("[neo] 未指定 --provider，默认使用 {picked}（/models 可切换）");
        picked
    } else {
        provider.to_string()
    };

    // 校验默认项存在
    if !entries.iter().any(|(i, _)| i.name == provider_name) {
        let mut names: Vec<&str> = entries.iter().map(|(i, _)| i.name.as_str()).collect();
        names.sort();
        eprintln!(
            "[neo] 未知 provider：{provider_name}（可选 {}）{}",
            names.join(" | "),
            if !deepseek_ok { "；deepseek 需要 DEEPSEEK_API_KEY" } else { "" }
        );
        return None;
    }

    match ModelRegistry::new(&provider_name, entries) {
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
