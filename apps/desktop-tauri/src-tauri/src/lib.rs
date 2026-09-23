//! Tauri 壳：只负责 spawn `neo app-server` 并把 stdio JSON-RPC 转给 WebView。
//!
//! **不依赖任何 neo-* crate** —— 对接走线协议（与编辑器/IDE 客户端同一路径），
//! 这样桌面复刻版验证的是「外部客户端能否无缝接入」，而不是同进程后门。

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

const PROTOCOL_VERSION: u32 = 1;
const RPC_TIMEOUT: Duration = Duration::from_secs(60);

/// 请求应答与进程状态分离：读线程只碰 `pending`，调用方碰 `stdin`，避免同一把锁死锁。
struct RpcMap {
    pending: Mutex<HashMap<i64, mpsc::Sender<Value>>>,
    next_id: Mutex<i64>,
    stdin: Mutex<Option<std::process::ChildStdin>>,
    child: Mutex<Option<Child>>,
    ready: Mutex<bool>,
}

impl Default for RpcMap {
    fn default() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            next_id: Mutex::new(0),
            stdin: Mutex::new(None),
            child: Mutex::new(None),
            ready: Mutex::new(false),
        }
    }
}

fn resolve_neo(explicit: Option<String>) -> Result<String, String> {
    if let Some(p) = explicit.filter(|s| !s.trim().is_empty()) {
        if PathBuf::from(&p).is_file() {
            return Ok(p);
        }
        return Err(format!("NEO_BIN 不存在：{p}"));
    }
    if let Ok(p) = std::env::var("NEO_BIN") {
        if !p.trim().is_empty() && PathBuf::from(&p).is_file() {
            return Ok(p);
        }
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    for rel in ["target/debug/neo", "target/release/neo"] {
        let cand = root.join(rel);
        if cand.is_file() {
            return Ok(cand.to_string_lossy().into_owned());
        }
    }
    if let Ok(paths) = std::env::var("PATH") {
        for dir in std::env::split_paths(&paths) {
            let cand = dir.join("neo");
            if cand.is_file() {
                return Ok(cand.to_string_lossy().into_owned());
            }
        }
    }
    Err(
        "找不到 neo 可执行文件。设置 NEO_BIN，或先在仓库根 `cargo build -p neo-code-cli`"
            .into(),
    )
}

fn rpc_send(map: &RpcMap, method: &str, params: Value) -> Result<Value, String> {
    let mut id_guard = map.next_id.lock().map_err(|_| "锁中毒")?;
    *id_guard += 1;
    let id = *id_guard;
    drop(id_guard);

    let (tx, rx) = mpsc::channel();
    map.pending
        .lock()
        .map_err(|_| "锁中毒")?
        .insert(id, tx);

    let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    {
        let mut stdin_g = map.stdin.lock().map_err(|_| "锁中毒")?;
        let stdin = stdin_g.as_mut().ok_or("app-server 未启动")?;
        let mut line = serde_json::to_string(&req).map_err(|e| e.to_string())?;
        line.push('\n');
        stdin
            .write_all(line.as_bytes())
            .map_err(|e| format!("写 stdin：{e}"))?;
        stdin.flush().map_err(|e| format!("flush：{e}"))?;
    }

    match rx.recv_timeout(RPC_TIMEOUT) {
        Ok(v) => {
            if let Some(err) = v.get("error") {
                let msg = err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("RPC error")
                    .to_string();
                return Err(msg);
            }
            Ok(v.get("result").cloned().unwrap_or(Value::Null))
        }
        Err(_) => {
            if let Ok(mut p) = map.pending.lock() {
                p.remove(&id);
            }
            Err(format!("RPC 超时：{method}"))
        }
    }
}

fn stop_child(map: &RpcMap) {
    if let Ok(mut child_g) = map.child.lock() {
        if let Some(mut c) = child_g.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
    if let Ok(mut s) = map.stdin.lock() {
        *s = None;
    }
    if let Ok(mut r) = map.ready.lock() {
        *r = false;
    }
    if let Ok(mut p) = map.pending.lock() {
        p.clear();
    }
}

#[tauri::command]
fn start_app_server(
    app: AppHandle,
    state: State<'_, Arc<RpcMap>>,
    bin: Option<String>,
    provider: Option<String>,
    workspace: Option<String>,
) -> Result<Value, String> {
    stop_child(&state);

    let neo = resolve_neo(bin)?;
    let provider = provider.unwrap_or_else(|| "mock".into());
    let mut cmd = Command::new(&neo);
    cmd.arg("app-server")
        .arg("--provider")
        .arg(&provider)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(ws) = workspace.filter(|s| !s.trim().is_empty()) {
        cmd.arg("--workspace").arg(ws);
    }

    let mut child = cmd.spawn().map_err(|e| format!("spawn {neo}：{e}"))?;
    let stdin = child.stdin.take().ok_or("无 stdin")?;
    let stdout = child.stdout.take().ok_or("无 stdout")?;
    let stderr = child.stderr.take().ok_or("无 stderr")?;

    *state.child.lock().map_err(|_| "锁中毒")? = Some(child);
    *state.stdin.lock().map_err(|_| "锁中毒")? = Some(stdin);

    {
        let app2 = app.clone();
        std::thread::spawn(move || {
            let r = BufReader::new(stderr);
            for line in r.lines().map_while(Result::ok) {
                let _ = app2.emit("neo-stderr", line);
            }
        });
    }

    let map = Arc::clone(&state);
    let app2 = app.clone();
    std::thread::spawn(move || {
        let r = BufReader::new(stdout);
        for line in r.lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                let _ = app2.emit("neo-protocol-log", line);
                continue;
            };
            if v.get("method").and_then(|m| m.as_str()) == Some("event") {
                let _ = app2.emit("neo-event", &v);
                continue;
            }
            if let Some(id) = v.get("id").and_then(|i| i.as_i64()) {
                if let Ok(mut p) = map.pending.lock() {
                    if let Some(tx) = p.remove(&id) {
                        let _ = tx.send(v);
                    }
                }
            }
        }
        let _ = app2.emit("neo-exit", Value::Null);
    });

    let init = json!({
        "protocol_version": PROTOCOL_VERSION,
        "client": { "name": "neo-desktop-tauri", "version": env!("CARGO_PKG_VERSION") },
    });
    let result = rpc_send(&state, "initialize", init)?;
    *state.ready.lock().map_err(|_| "锁中毒")? = true;
    Ok(result)
}

#[tauri::command]
fn rpc_call(
    state: State<'_, Arc<RpcMap>>,
    method: String,
    params: Option<Value>,
) -> Result<Value, String> {
    let ready = *state.ready.lock().map_err(|_| "锁中毒")?;
    if !ready {
        return Err("app-server 未 initialize".into());
    }
    rpc_send(&state, &method, params.unwrap_or_else(|| json!({})))
}

#[tauri::command]
fn stop_app_server(state: State<'_, Arc<RpcMap>>) -> Result<(), String> {
    let _ = rpc_send(&state, "shutdown", json!({}));
    stop_child(&state);
    Ok(())
}

/// 工作区文件树（D12）：有界遍历，跳过忽略目录。
/// 返回相对路径列表（目录以 `/` 结尾），排序稳定。
#[tauri::command]
fn list_workspace(root: Option<String>) -> Result<Value, String> {
    let base = match root {
        Some(p) if !p.trim().is_empty() => PathBuf::from(p),
        _ => std::env::current_dir().map_err(|e| e.to_string())?,
    };
    if !base.is_dir() {
        return Err(format!("不是目录：{}", base.display()));
    }
    const MAX_ENTRIES: usize = 800;
    const MAX_DEPTH: usize = 8;
    const SKIP: &[&str] = &[
        ".git", "node_modules", "target", "dist", ".neo", ".cache", "__pycache__",
        ".DS_Store",
    ];
    let mut out: Vec<String> = Vec::new();
    let mut truncated = false;

    fn walk(
        dir: &PathBuf,
        base: &PathBuf,
        rel: &str,
        depth: usize,
        max_depth: usize,
        max_entries: usize,
        skip: &[&str],
        out: &mut Vec<String>,
        truncated: &mut bool,
    ) {
        if depth > max_depth || out.len() >= max_entries {
            *truncated = true;
            return;
        }
        let mut entries: Vec<_> = match std::fs::read_dir(dir) {
            Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
            Err(_) => return,
        };
        entries.sort_by_key(|e| {
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            (
                !is_dir,
                e.file_name().to_string_lossy().to_lowercase(),
            )
        });
        for e in entries {
            if out.len() >= max_entries {
                *truncated = true;
                return;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if skip.contains(&name.as_str()) {
                continue;
            }
            let path = e.path();
            let child_rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                out.push(format!("{child_rel}/"));
                walk(
                    &path,
                    base,
                    &child_rel,
                    depth + 1,
                    max_depth,
                    max_entries,
                    skip,
                    out,
                    truncated,
                );
            } else {
                out.push(child_rel);
            }
        }
    }

    walk(
        &base,
        &base,
        "",
        0,
        MAX_DEPTH,
        MAX_ENTRIES,
        SKIP,
        &mut out,
        &mut truncated,
    );
    Ok(json!({
        "root": base.to_string_lossy(),
        "entries": out,
        "truncated": truncated,
    }))
}

/// 读工作区单文件（有界）：点文件预览内容（D12）。
#[tauri::command]
fn read_workspace_file(root: Option<String>, path: String) -> Result<Value, String> {
    if path.contains('\0') || path.contains("..") {
        return Err("非法路径".into());
    }
    let base = match root {
        Some(p) if !p.trim().is_empty() => PathBuf::from(p),
        _ => std::env::current_dir().map_err(|e| e.to_string())?,
    };
    let full = base.join(path.trim_start_matches('/'));
    let full_c = full.canonicalize().map_err(|e| e.to_string())?;
    let base_c = base.canonicalize().map_err(|e| e.to_string())?;
    if !full_c.starts_with(&base_c) {
        return Err("路径越出工作区".into());
    }
    if !full_c.is_file() {
        return Err("不是文件".into());
    }
    let meta = std::fs::metadata(&full_c).map_err(|e| e.to_string())?;
    const MAX_BYTES: u64 = 256 * 1024;
    if meta.len() > MAX_BYTES {
        return Err(format!("文件过大（{} bytes > 256KiB）", meta.len()));
    }
    // 粗略二进制探测
    let bytes = std::fs::read(&full_c).map_err(|e| e.to_string())?;
    if bytes.iter().take(8000).any(|b| *b == 0) {
        return Ok(json!({
            "path": path,
            "binary": true,
            "content": null,
            "bytes": meta.len(),
        }));
    }
    let content = String::from_utf8_lossy(&bytes).into_owned();
    Ok(json!({
        "path": path,
        "binary": false,
        "content": content,
        "bytes": meta.len(),
        "truncated": false,
    }))
}

/// 敏感内容粗检（D12 Wiki 底线）：命中则整篇排除，不回显原文。
fn looks_sensitive(text: &str) -> Option<&'static str> {
    if text.contains("-----BEGIN") && text.contains("PRIVATE KEY") {
        return Some("private_key");
    }
    for (pfx, kind) in [
        ("sk-", "api_key"),
        ("ghp_", "api_key"),
        ("gho_", "api_key"),
        ("github_pat_", "api_key"),
        ("xoxb-", "api_key"),
        ("AKIA", "aws_key"),
        ("AIza", "api_key"),
        ("sk_live_", "api_key"),
        ("sk_test_", "api_key"),
    ] {
        if text.contains(pfx) {
            return Some(kind);
        }
    }
    None
}

/// Repo Wiki（D12）：根目录入口 Markdown + `docs/**/*.md`，过敏感闸。
#[tauri::command]
fn list_repo_wiki(root: Option<String>) -> Result<Value, String> {
    let base = match root {
        Some(p) if !p.trim().is_empty() => PathBuf::from(p),
        _ => std::env::current_dir().map_err(|e| e.to_string())?,
    };
    const MAX_DOCS: usize = 40;
    const MAX_BYTES: usize = 128 * 1024;
    let mut candidates: Vec<PathBuf> = Vec::new();
    for name in [
        "README.md", "AGENTS.md", "CONTRIBUTING.md", "ARCHITECTURE.md",
        "CHANGELOG.md", "PROJECT_MEMORY.md", "LICENSE.md",
    ] {
        let p = base.join(name);
        if p.is_file() {
            candidates.push(p);
        }
    }
    let docs = base.join("docs");
    if docs.is_dir() {
        collect_md(&docs, &docs, 0, 4, MAX_DOCS, &mut candidates);
    }

    let mut items = Vec::new();
    let mut excluded = 0usize;
    let mut truncated = false;
    for (i, path) in candidates.into_iter().enumerate() {
        if items.len() >= MAX_DOCS {
            truncated = true;
            break;
        }
        let rel = path
            .strip_prefix(&base)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if bytes.len() > MAX_BYTES {
            excluded += 1;
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        if let Some(_kind) = looks_sensitive(&text) {
            // 不回显密钥类型细节里的内容，只计数（与规格一致：不回显原文）
            excluded += 1;
            continue;
        }
        let title = text
            .lines()
            .find(|l| l.starts_with("# "))
            .map(|l| l.trim_start_matches("# ").trim().to_string())
            .unwrap_or_else(|| rel.clone());
        items.push(json!({
            "path": rel,
            "title": title,
            "bytes": bytes.len(),
        }));
        let _ = i;
    }
    Ok(json!({
        "root": base.to_string_lossy(),
        "items": items,
        "excluded": excluded,
        "truncated": truncated,
    }))
}

fn collect_md(
    dir: &PathBuf,
    base: &PathBuf,
    depth: usize,
    max_depth: usize,
    max: usize,
    out: &mut Vec<PathBuf>,
) {
    if depth > max_depth || out.len() >= max {
        return;
    }
    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return,
    };
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        if out.len() >= max {
            return;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let path = e.path();
        if path.is_dir() {
            collect_md(&path, base, depth + 1, max_depth, max, out);
        } else if name.to_lowercase().ends_with(".md") {
            out.push(path);
        }
    }
}

/// 「不在项目中工作」的固定工作区：`~/.neo/no-project`（会话库在此，不绑仓库）。
/// 不用进程 cwd —— 打包后 cwd 是 App 目录，会话会落错地方。
#[tauri::command]
fn no_project_dir() -> Result<String, String> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_err(|_| "无法确定用户主目录")?;
    let p = std::path::PathBuf::from(home)
        .join(".neo")
        .join("no-project");
    std::fs::create_dir_all(&p).map_err(|e| format!("创建 {p:?}：{e}"))?;
    Ok(p.to_string_lossy().into_owned())
}

/// 系统「打开文件夹」对话框。**async + spawn_blocking**：同步 osascript 会堵主线程导致 WebView 黑屏。
#[tauri::command]
async fn pick_folder() -> Result<String, String> {
    if !cfg!(target_os = "macos") {
        return Err("当前平台暂用路径输入；macOS 请用打开文件夹".into());
    }
    tauri::async_runtime::spawn_blocking(|| {
        let out = std::process::Command::new("osascript")
            .args([
                "-e",
                r#"POSIX path of (choose folder with prompt "选择工作区文件夹")"#,
            ])
            .output()
            .map_err(|e| format!("启动系统对话框失败：{e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            if err.to_lowercase().contains("cancel") || out.status.code() == Some(1) {
                return Err("已取消".into());
            }
            return Err(format!("打开文件夹失败：{err}"));
        }
        let path = String::from_utf8_lossy(&out.stdout)
            .trim()
            .trim_end_matches('\n')
            .to_string();
        if path.is_empty() {
            return Err("已取消".into());
        }
        Ok(path.trim_end_matches('/').to_string())
    })
    .await
    .map_err(|e| format!("任务失败：{e}"))?
}

/// 抓取网页 HTML（curl），供浏览器点选前把跨域页变成可注入的 srcdoc。
#[tauri::command]
async fn fetch_url(url: String) -> Result<String, String> {
    let u = url.trim().to_string();
    if !(u.starts_with("https://") || u.starts_with("http://")) {
        return Err("仅允许 http/https".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let out = std::process::Command::new("curl")
            .args(["-sL", "--max-time", "20", "--compressed", "-A", "NEO-Desktop/0.1", &u])
            .output()
            .map_err(|e| format!("curl 失败：{e}"))?;
        if !out.status.success() {
            return Err(format!("HTTP 失败：{}", out.status));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    })
    .await
    .map_err(|e| format!("任务失败：{e}"))?
}

/// 用系统默认浏览器打开 URL（仅 http/https/file）。
#[tauri::command]
async fn open_url(url: String) -> Result<(), String> {
    let u = url.trim().to_string();
    let ok = u.starts_with("https://")
        || u.starts_with("http://")
        || u.starts_with("file:///")
        || (u.starts_with('/') && !u.contains(".."));
    if !ok {
        return Err("仅允许 http/https/file 路径".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(target_os = "macos")]
        let mut cmd = std::process::Command::new("open");
        #[cfg(not(target_os = "macos"))]
        let mut cmd = std::process::Command::new("xdg-open");
        cmd.arg(&u);
        let out = cmd.status().map_err(|e| format!("打开失败：{e}"))?;
        if !out.success() {
            return Err("系统打开命令失败".into());
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("任务失败：{e}"))?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let map = Arc::new(RpcMap::default());
    tauri::Builder::default()
        .manage(map)
        .invoke_handler(tauri::generate_handler![
            start_app_server,
            rpc_call,
            stop_app_server,
            list_workspace,
            read_workspace_file,
            list_repo_wiki,
            no_project_dir,
            pick_folder,
            open_url,
            fetch_url
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
