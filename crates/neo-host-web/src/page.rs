//! 内置前端页面（单文件，零外部资源）
//!
//! 刻意保持极小：它的作用是**让 Web 宿主可用且有说服力**，不是做一个产品级 UI。
//! 交互与 TUI 对齐（提交任务、看事件、审批 y/n），从而证明"同一内核、多宿主"
//! 不是纸面说法。

/// 内置页面 HTML。用 SSE 收事件，用 fetch 提交任务。
pub const INDEX_HTML: &str = r#"<!doctype html>
<html lang="zh">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>NEO</title>
<style>
  :root { color-scheme: dark; }
  body { margin:0; font:14px/1.6 ui-monospace,SFMono-Regular,Menlo,monospace;
         background:#0b0f14; color:#e6edf3; }
  header { padding:10px 16px; border-bottom:1px solid #1f2937; display:flex; gap:12px; align-items:center; }
  header b { color:#7dd3fc; }
  .pill { font-size:11px; padding:2px 8px; border:1px solid #334155; border-radius:999px; color:#94a3b8; }
  #log { padding:12px 16px; white-space:pre-wrap; word-break:break-word; min-height:60vh; }
  .tool { color:#7dd3fc; } .err { color:#f87171; } .ask { color:#fbbf24; }
  .me { color:#e6edf3; border-left:2px solid #9d7cd8; padding-left:8px; margin:4px 0; }
  .dim { color:#64748b; }
  footer { position:sticky; bottom:0; border-top:1px solid #1f2937; background:#0b0f14; padding:10px 16px; display:flex; gap:8px; }
  input { flex:1; background:#111827; color:#e6edf3; border:1px solid #334155; border-radius:8px; padding:8px 10px; font:inherit; }
  button { background:#1d4ed8; color:#fff; border:0; border-radius:8px; padding:8px 14px; font:inherit; cursor:pointer; }
  button.ghost { background:#1f2937; }
</style>
</head>
<body>
<header>
  <b>NEO</b><span class="pill" id="state">连接中…</span>
  <span class="pill" id="subs">订阅 0</span>
  <button class="ghost" id="clear" style="margin-left:auto">清屏</button>
</header>
<div id="log"></div>
<footer>
  <input id="task" placeholder="输入任务，回车提交（写操作会要求审批）" autocomplete="off">
  <button id="send">发送</button>
</footer>
<script>
const log = document.getElementById('log');
const state = document.getElementById('state');
const subs = document.getElementById('subs');
const input = document.getElementById('task');

// 待审批的调用 id：内核挂起后必须由用户应答
let pendingApproval = null;

function line(text, cls) {
  const d = document.createElement('div');
  if (cls) d.className = cls;
  d.textContent = text;
  log.appendChild(d);
  log.scrollTop = log.scrollHeight;
}

async function send() {
  const text = input.value.trim();
  if (!text) return;
  input.value = '';
  line('› ' + text, 'dim');
  try {
    const r = await fetch('./api/turn', { method: 'POST', body: text });
    if (!r.ok) line('提交失败：' + (await r.text()), 'err');
  } catch (e) { line('提交失败：' + e, 'err'); }
}
document.getElementById('send').onclick = send;
document.getElementById('clear').onclick = () => { log.textContent = ''; };
input.addEventListener('keydown', (e) => {
  if (e.key !== 'Enter') return;
  // 有未决审批时，Enter 提交的是审批应答
  if (pendingApproval) {
    const answer = input.value.trim().toLowerCase();
    if (answer !== 'y' && answer !== 'n') { line('请输入 y 或 n', 'ask'); return; }
    input.value = '';
    line((answer === 'y' ? '已批准 ' : '已拒绝 ') + pendingApproval, 'ask');
    fetch('./api/approve?id=' + encodeURIComponent(pendingApproval) + '&allow=' + (answer === 'y'));
    pendingApproval = null;
    return;
  }
  send();
});

// SSE：接收事件流
const es = new EventSource('./api/events');
es.onopen = () => { state.textContent = '已连接'; };
es.onerror = () => { state.textContent = '连接中断，浏览器会自动重连'; };
es.onmessage = (ev) => {
  let m;
  try { m = JSON.parse(ev.data); } catch { return; }
  const k = m.kind || Object.keys(m)[0];
  if (k === 'subscribed') { line('已连上事件流', 'dim'); return; }
  if (k === 'user_submitted') { line('┃ ' + (m.text ?? ''), 'me'); return; }
  if (k === 'agent_message_delta') { appendDelta(m); return; }
  if (k === 'agent_message_done') { line(''); return; }
  if (k === 'tool_call_begin') { line('▸ 调用工具 ' + (m.name ?? ''), 'tool'); return; }
  if (k === 'tool_call_end') { line('▸ 工具结束 exit ' + (m.exit_code ?? '?'), 'tool'); return; }
  if (k === 'approval_request') {
    pendingApproval = m.id;
    line('⚠ 需要审批：' + (m.detail ?? '') + '  （输入 y 或 n 后回车）', 'ask');
    return;
  }
  if (k === 'error') { line('✗ ' + (m.message ?? ''), 'err'); return; }
  if (k === 'turn_complete') {
    line('· 本轮完成（' + (m.input_tokens ?? 0) + ' in / ' + (m.output_tokens ?? 0) + ' out）', 'dim');
    return;
  }
};

// 流式增量合并到同一行（与 TUI 的 pending_text 语义一致）
let deltaEl = null;
function appendDelta(m) {
  if (!deltaEl) { deltaEl = document.createElement('div'); log.appendChild(deltaEl); }
  deltaEl.textContent += (m.delta ?? '');
  log.scrollTop = log.scrollHeight;
}
</script>
</body>
</html>
"#;
