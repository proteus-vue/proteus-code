/**
 * NEO app-server 线协议 —— 由 `cargo test -p neo-host-appserver --features schema export_schema` 生成。
 * 不要手改：改 Rust 类型后重新生成，否则门禁 check_protocol_schema.py 会红。
 * protocol version: 1
 */

export const PROTOCOL_VERSION = 1 as const;

export type ProtocolVersion = typeof PROTOCOL_VERSION;

/** initialize 响应里的 methods 全表。 */
export const METHODS = [
  "initialize",
  "turn/start",
  "turn/begin",
  "turn/pump",
  "turn/interrupt",
  "command/exec",
  "approval/respond",
  "approval/respondStep",
  "user_input/respond",
  "session/configure",
  "session/compact",
  "session/fork",
  "session/rewind",
  "goal/set",
  "goal/pause",
  "goal/resume",
  "goal/advance",
  "goal/clear",
  "shutdown",
  "thread/list",
  "thread/get",
  "thread/resume",
  "thread/create",
  "thread/delete",
  "thread/rename",
  "thread/history",
  "thread/export",
  "thread/goal/get",
  "tools/list",
  "models/list",
  "git/info",
  "command/exec/write",
  "command/exec/resize",
  "command/exec/terminate",
  "thread/goal/set",
  "thread/goal/clear",
] as const;
export type MethodName = (typeof METHODS)[number];

/** JSON-RPC 服务端错误码（data 可带 allowed / unknown 等细节）。 */
export const ERROR_CODES = {
  PARSE_ERROR: -32700,
  INVALID_REQUEST: -32600,
  METHOD_NOT_FOUND: -32601,
  INVALID_PARAMS: -32602,
  KERNEL_ERROR: -32000,
  NOT_INITIALIZED: -32001,
  ALREADY_INITIALIZED: -32002,
  VERSION_MISMATCH: -32003,
} as const;

/** 方法 → 合法参数键（与服务端严格校验一致）。 */
export const METHOD_PARAMS: Record<string, readonly string[]> = {
  "initialize": ["protocol_version", "client"],
  "turn/start": ["text"],
  "turn/begin": ["text"],
  "turn/pump": [],
  "turn/interrupt": [],
  "command/exec": ["command"],
  "approval/respond": ["id", "decision", "reason"],
  "approval/respondStep": ["id", "decision", "reason"],
  "user_input/respond": ["id", "response"],
  "session/configure": ["exec_mode", "sandbox_mode", "approval_policy", "model", "token_budget"],
  "session/compact": [],
  "session/fork": [],
  "session/rewind": ["turns"],
  "goal/set": ["goal"],
  "goal/pause": ["goal_id"],
  "goal/resume": ["goal_id"],
  "goal/advance": [],
  "goal/clear": [],
  "shutdown": [],
  "thread/list": [],
  "thread/get": ["id"],
  "thread/resume": ["id"],
  "thread/create": [],
  "thread/delete": ["id"],
  "thread/rename": ["id", "title"],
  "thread/history": ["id"],
  "thread/export": ["id", "format"],
  "thread/goal/get": [],
  "tools/list": [],
  "models/list": [],
  "git/info": ["cwd"],
  "command/exec/write": ["session_id", "data"],
  "command/exec/resize": ["session_id", "cols", "rows"],
  "command/exec/terminate": ["session_id"],
  "thread/goal/set": ["goal"],
  "thread/goal/clear": [],
};

export type ApprovalParams = { id: string, decision: Decision, reason: string | null, };
export type ApprovalPolicy = "untrusted" | "on_request" | "on_failure" | "never";
export type ClientInfoWire = { name: string, version: string | null, };
export type CommandParams = { command: string, };
export type ContextRef = { kind: RefKind, target: string, 
/**
 * 行范围（1 基，闭区间）。`None` = 整个文件。
 *
 * 单独建模而不是塞进 `target` 字符串：`src/a.rs#12-40` 里的
 * "#12-40" 是**结构化信息**（起止行），下游（提示词拼装、UI 高亮）
 * 需要数值而不是再解析一遍字符串。
 */
lines: [number, number] | null, };
export type Decision = "allow" | "allow_always" | "deny";
export type EventMsg = { "user_submitted": { text: string, } } | { "refs_resolved": { 
/**
 * 人类可读的一行摘要（用户可见事实）：解析成功了几条、哪条失败。
 */
summary: Array<string>, 
/**
 * 注入模型请求的完整上下文块。
 */
block: string, } } | { "session_configured": { session_id: string, } } | { "instructions_loaded": { sources: Array<string>, block: string, truncated: boolean, } } | { "model_switched": { model: string, context_limit: number, } } | { "turn_started": { turn_id: string, } } | { "agent_message_delta": { delta: string, } } | { "agent_message_done": { text: string, } } | { "reasoning_delta": { delta: string, } } | { "tool_call_begin": { id: string, name: string, 
/**
 * 调用参数。**必须落日志** —— 模型下次请求要带完整的 assistant
 * tool_call（含 arguments），否则会话无法从日志重建、
 * 跨进程续聊时请求非法。这也正是 AGENTS.md「模型可见即已落日志」
 * 那条约束的要求。
 */
arguments: unknown, } } | { "tool_call_end": { id: string, exit_code: number, 
/**
 * 工具输出。**必须进事件流** —— 否则用户只看到 `✓ bash exit 0`
 * 而看不到命令打印了什么，等于无法判断这步到底做了什么。
 * 内核已按上限截断，`truncated` 如实标注。
 */
stdout: string, stderr: string, truncated: boolean, } } | { "approval_request": { id: string, detail: string, kind: string, } } | { "user_input_request": { id: string, prompt: string, } } | { "image_attached": { id: string, path: string, mime: string, bytes: number, } } | { "patch_proposed": { path: string, diff: string, } } | { "checkpoint_saved": { checkpoint_id: string, } } | { "file_changed": { path: string, additions: number, deletions: number, } } | { "files_changed": { files: Array<FileChange>, } } | { "context_compacted": { removed_messages: number, summary: string, } } | { "rewound": { turns: number, removed_messages: number, files_kept: number, } } | { "todo_updated": { items: Array<TodoEntry>, } } | { "goal_progress": { goal_id: string, done: number, total: number, } } | { "goal_updated": { snapshot: GoalSnapshot, } } | { "goal_cleared": { goal_id: string, } } | { "error": { message: string, } } | { "turn_complete": { input_tokens: number, output_tokens: number, } } | "shutdown_complete";
export type EventNotification = { seq: number, kind: string, payload: unknown, };
export type ExecMode = "plan" | "confirm_before" | "default" | "auto_edit" | "full_access";
export type ExecResizeParams = { session_id: string, cols: number, rows: number, };
export type ExecTerminateParams = { session_id: string, };
export type ExecWriteParams = { session_id: string, data: string, };
export type Fact = { "user_said": string } | { "refs_resolved": Array<string> } | { "assistant_said": string } | { "assistant_thought": string } | { "tool_finished": { name: string, exit_code: number, stdout: string, stderr: string, truncated: boolean, 
/**
 * 调用参数原文（来自 ToolCallBegin.arguments 的 JSON 文本）。
 * 宿主据此在工具行上显示"执行了什么"（如 bash 的命令），
 * 不用展开详情。派生视图，非事件流内容。
 */
args: string | null, } } | { "approval_needed": { detail: string, } } | { "todo_list": Array<TodoEntry> } | { "context_compacted": { removed_messages: number, } } | { "rewound": { turns: number, removed_messages: number, files_kept: number, } } | { "files_changed": Array<FileChange> } | { "patch_preview": { path: string, diff: string, } } | { "failed": string } | { "turn_finished": { input_tokens: number, output_tokens: number, } } | { "session_ready": { session_id: string, } } | { "instructions_loaded": { sources: Array<string>, truncated: boolean, } } | { "model_switched": { model: string, context_limit: number, } } | { "goal": string } | { "goal_cleared": string };
export type FileChange = { path: string, additions: number, deletions: number, };
export type GoalIdParams = { goal_id: string, };
export type GoalParams = { goal: string, };
export type GoalPhase = "plan" | "code" | "review" | "learn" | "done";
export type GoalSnapshot = { 
/**
 * 确定性编号（`goal-N`，按编排器内计数器派生，不用随机/时钟）
 */
goal_id: string, 
/**
 * 用户设定的目标原文
 */
goal: string, 
/**
 * 暂停中（不推进，但目标保留）
 */
paused: boolean, 
/**
 * 引擎触发停止条件后的原因（非空时不再推进）
 */
stopped: string | null, subtasks: Array<GoalSubtask>, 
/**
 * 已消耗的编排步数
 */
iterations: number, 
/**
 * 连续失败计数（审查判停的依据 —— 回放重建后判停行为必须一致，
 * 所以它必须在快照里，而不能只活在内存里）
 */
consecutive_failures: number, 
/**
 * 还需要多少个子任务轮（**下限估计**：审查失败的重试会让实际更多）。
 * 宿主据此决定要不要继续 `GoalAdvance`。
 */
turns_remaining: number, 
/**
 * 目标已消耗的 token 预算
 */
budget_used: number, };
export type GoalSubtask = { id: number, title: string, phase: GoalPhase, retries: number, };
export type InitializeParamsWire = { protocol_version: number | null, client: ClientInfoWire | null, };
export type MethodDoc = { name: string, 
/**
 * 无参方法为空数组。
 */
params: Array<string>, 
/**
 * 是否在握手前可用。
 */
handshake_only: boolean, };
export type Op = { "user_turn": { text: string, refs: Array<ContextRef>, } } | { "begin_turn": { text: string, refs: Array<ContextRef>, } } | "pump" | { "shell": { command: string, } } | "interrupt" | { "approve": { id: string, decision: Decision, reason: string | null, } } | { "approve_step": { id: string, decision: Decision, reason: string | null, } } | { "respond_user_input": { id: string, response: string, } } | { "configure_session": { patch: SessionPatch, } } | "compact" | "fork" | { "rewind": { turns: number, } } | { "goal_set": { goal: string, } } | { "goal_pause": { goal_id: string, } } | { "goal_resume": { goal_id: string, } } | "goal_advance" | "goal_clear" | "shutdown";
export type RefKind = "file" | "session" | "command" | "skill";
export type RewindParams = { turns: number, };
export type RpcErrorObject = { code: number, message: string, data: unknown, };
export type RpcErrorResponse = { jsonrpc: string, id: unknown, error: RpcErrorObject, };
export type RpcNotification = { jsonrpc: string, method: string, params: unknown, };
export type RpcRequest = { jsonrpc: string, 
/**
 * 请求 id（字符串或整数）。
 */
id: unknown, method: string, params: unknown, };
export type RpcResponse = { jsonrpc: string, id: unknown, result: unknown, };
export type SandboxMode = "read_only" | "workspace_write" | "danger_full_access";
export type SessionPatch = { exec_mode: ExecMode | null, sandbox_mode: SandboxMode | null, approval_policy: ApprovalPolicy | null, model: string | null, 
/**
 * 会话级 token 预算（护栏 #9）。`Some(0)` = 清除预算（不限）。
 *
 * 缺省 `None` = **不改**现有预算（与其它字段同语义）。
 * 要"不限"必须显式传 `0`，不能靠缺省——否则桌面端漏传会静默抹掉用户设过的预算。
 */
token_budget: number | null, };
export type TextParams = { text: string, };
export type ThreadSummary = { id: string, 
/**
 * 标题（取自第一条用户消息；取不到则等于 id）
 */
title: string, 
/**
 * 标题是否来自真实内容（false = id 兜底，UI 应弱化显示）
 */
has_title: boolean, 
/**
 * 日志条数（粗略规模）
 */
records: number, 
/**
 * 文件字节数
 */
bytes: number, 
/**
 * 累计改动（增, 删）；从未改动为 null。取自**最后一个** files_changed。
 */
changes: [number, number] | null, 
/**
 * idle / failed / interrupted / empty
 */
state: string, };
export type TodoEntry = { content: string, status: TodoStatus, };
export type TodoStatus = "pending" | "in_progress" | "completed";
export type ToolOutput = { exit_code: number, stdout: string, stderr: string, truncated: boolean, };
export type UserInputParams = { id: string, response: string, };
