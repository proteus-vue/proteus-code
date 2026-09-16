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
  #goalbar { display:flex; gap:8px; padding:8px 16px; border-bottom:1px solid #1f2937; align-items:flex-end; }
  #goalbar textarea { flex:1; background:#111827; color:#e6edf3; border:1px solid #334155; border-radius:8px; padding:6px 10px; font:inherit; resize:vertical; }
  .goal { color:#7dd3fc; }
</style>
</head>
<body>
<header>
  <b>NEO</b><span class="pill" id="state">连接中…</span>
  <span class="pill" id="subs">订阅 0</span>
  <button class="ghost" id="clear" style="margin-left:auto">清屏</button>
</header>
<div id="goalbar">
  <textarea id="goal" rows="2" placeholder="目标（每行一个子任务，留空行分隔）"></textarea>
  <button id="goal-set">开始</button>
  <button class="ghost" id="goal-pause">暂停</button>
  <button class="ghost" id="goal-resume">恢复</button>
  <button class="ghost" id="goal-clear">清除</button>
</div>
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

// 访问令牌：从 URL fragment（`#token=...`）读取。
// 放 fragment 而不是 query —— fragment 不发给服务器、不进 Referer、
// 也不进服务端请求日志，令牌只留在本地地址栏。宿主打印的 URL 自带它。
const TOKEN = new URLSearchParams(location.hash.slice(1)).get('token') || '';
// 用裸地址打开时明确告知该怎么办，而不是让每个请求静默 401
// （脚本在 body 末尾，log 已就绪，line 是提升的函数声明）
if (!TOKEN) line('未检测到访问令牌：请用启动时打印的完整 URL 打开（形如 http://127.0.0.1:端口/#token=…）', 'err');
// 所有 /api/* 都要令牌；EventSource 不能设请求头，故统一走 query
function apiPath(p) {
  if (!TOKEN) return p;
  return p + (p.includes('?') ? '&' : '?') + 'token=' + encodeURIComponent(TOKEN);
}

// 待审批的调用 id：内核挂起后必须由用户应答
let pendingApproval = null;
// 目标编排：goal = 最新快照（每次 goal_updated 整体覆盖）
let goal = null;
const goalBox = document.getElementById('goal');

function phaseName(p) {
  return { plan:'计划', code:'执行', review:'审查', learn:'复盘', done:'完成' }[p] ?? p;
}
function goalSummary(s) {
  const done = s.subtasks.filter(t => t.phase === 'done').length;
  const head = '🎯 ' + s.goal_id + ': ' + done + '/' + s.subtasks.length + ' 完成';
  if (s.stopped) return head + ' · 已停止：' + s.stopped;
  const cur = s.subtasks.find(t => t.phase !== 'done');
  const tail = cur ? ' 当前：' + cur.title + '（' + phaseName(cur.phase) + '）' : ' 全部完成';
  return head + tail + (s.paused ? ' · 已暂停' : '');
}
// 推进的**单一驱动源**：goal_updated 快照。决策永远基于最新状态 ——
// 若改由 turn_complete 驱动，本地快照会比事件流晚一步，最终轮会多发
// 一次 advance 并报"没有待执行"（浏览器实测抓到的竞态）。
// 一次 advance = 一个完整子任务轮；停止条件由引擎保证。
async function maybeGoalAdvance() {
  if (!goal || goal.paused || goal.stopped || !(goal.turns_remaining > 0)) return;
  try { await fetch(apiPath('./api/goal?action=advance')); }
  catch (e) { line('目标推进失败：' + e, 'err'); }
}
async function goalAction(action) {
  try {
    const r = await fetch(apiPath('./api/goal?action=' + action));
    if (!r.ok) line('目标操作失败：' + (await r.text()), 'err');
  } catch (e) { line('目标操作失败：' + e, 'err'); }
}
document.getElementById('goal-set').onclick = async () => {
  const text = goalBox.value.trim();
  if (!text) { line('目标为空', 'ask'); return; }
  try {
    const r = await fetch(apiPath('./api/goal'), { method: 'POST', body: text });
    if (!r.ok) line('目标提交失败：' + (await r.text()), 'err');
  } catch (e) { line('目标提交失败：' + e, 'err'); }
};
document.getElementById('goal-pause').onclick = () => goalAction('pause');
document.getElementById('goal-resume').onclick = () => goalAction('resume');
document.getElementById('goal-clear').onclick = () => goalAction('clear');

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
    const r = await fetch(apiPath('./api/turn'), { method: 'POST', body: text });
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
    fetch(apiPath('./api/approve?id=' + encodeURIComponent(pendingApproval) + '&allow=' + (answer === 'y')));
    pendingApproval = null;
    return;
  }
  send();
});

// SSE：接收事件流
const es = new EventSource(apiPath('./api/events'));
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
  if (k === 'goal_updated') {
    goal = m.snapshot;
    line(goalSummary(goal), 'goal');
    maybeGoalAdvance();
    return;
  }
  if (k === 'goal_cleared') {
    goal = null;
    line('🎯 目标 ' + (m.goal_id ?? '') + ' 已清除', 'dim');
    return;
  }
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
