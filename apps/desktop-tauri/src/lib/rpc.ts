import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ApprovalState,
  Decision,
  InitializeResult,
  ModelsResult,
  ThreadSummary,
  WireEvent,
} from "./protocol";

export async function startServer(opts?: {
  bin?: string;
  provider?: string;
  workspace?: string;
}): Promise<InitializeResult> {
  return invoke<InitializeResult>("start_app_server", {
    bin: opts?.bin ?? null,
    provider: opts?.provider ?? "mock",
    workspace: opts?.workspace ?? null,
  });
}

export function rpc<T = unknown>(
  method: string,
  params?: Record<string, unknown>,
): Promise<T> {
  return invoke<T>("rpc_call", { method, params: params ?? {} });
}

export function stopServer(): Promise<void> {
  return invoke("stop_app_server");
}

/** 「不在项目中工作」固定目录 `~/.neo/no-project` */
export async function noProjectDir(): Promise<string> {
  return invoke<string>("no_project_dir");
}

/** 系统「打开文件夹」对话框；取消 reject "已取消" */
export async function pickFolder(): Promise<string> {
  return invoke<string>("pick_folder");
}

export async function listThreads(): Promise<ThreadSummary[]> {
  const r = await rpc<{ threads?: ThreadSummary[] }>("thread/list");
  return r.threads ?? [];
}

export async function createThread(): Promise<string> {
  const r = await rpc<{ id?: string }>("thread/create");
  return r.id ?? "";
}

export async function resumeThread(id: string): Promise<unknown> {
  return rpc("thread/resume", { id });
}

export async function deleteThread(id: string): Promise<boolean> {
  const r = await rpc<{ removed?: boolean }>("thread/delete", { id });
  return Boolean(r.removed);
}

/** 用户改名：同一会话 JSONL 追加 op/set_title。 */
export async function renameThread(
  id: string,
  title: string,
): Promise<{ id: string; title: string; has_title?: boolean }> {
  return rpc("thread/rename", { id, title });
}

/** 历史事实投影（facts_of），不切换会话。 */
export async function threadHistory(id?: string): Promise<import("./protocol").HistoryResult> {
  return rpc("thread/history", id ? { id } : {});
}

export async function startTurn(text: string): Promise<unknown> {
  return rpc("turn/start", { text });
}

export async function interruptTurn(): Promise<unknown> {
  return rpc("turn/interrupt");
}

/** 分叉当前会话（协议 `session/fork`）。 */
export async function forkThread(): Promise<unknown> {
  return rpc("session/fork", {});
}

/** 回退最近 N 个用户轮（编辑历史 / 对话回退）。 */
export async function rewindTurns(turns: number): Promise<unknown> {
  return rpc("session/rewind", { turns });
}

export async function respondApproval(
  id: string,
  decision: Decision,
  reason?: string | null,
): Promise<unknown> {
  return rpc("approval/respond", {
    id,
    decision,
    reason: reason ?? null,
  });
}

export async function listModels(): Promise<ModelsResult> {
  return rpc<ModelsResult>("models/list");
}

export type ToolInfo = {
  name: string;
  description?: string;
  parameters?: unknown;
};

/** `tools/list` —— 含 `agent_*` 子代理（装配时从 .neo/agents 加载）。 */
export async function listTools(): Promise<{ tools?: ToolInfo[] }> {
  return rpc("tools/list", {});
}

export type ExecMode =
  | "plan"
  | "confirm_before"
  | "default"
  | "auto_edit"
  | "full_access";

/** 会话配置（SessionPatch 子集）；字段名 = 线协议 snake_case。 */
export async function configureSession(patch: {
  exec_mode?: ExecMode;
  model?: string;
  token_budget?: number;
}): Promise<unknown> {
  return rpc("session/configure", patch);
}

export async function gitInfo(): Promise<Record<string, unknown>> {
  return rpc("git/info", {});
}

export async function goalSet(goal: string): Promise<unknown> {
  return rpc("goal/set", { goal });
}
export async function goalPause(goalId: string): Promise<unknown> {
  return rpc("goal/pause", { goal_id: goalId });
}
export async function goalResume(goalId: string): Promise<unknown> {
  return rpc("goal/resume", { goal_id: goalId });
}
export async function goalClear(): Promise<unknown> {
  return rpc("goal/clear", {});
}
export async function goalAdvance(): Promise<unknown> {
  return rpc("goal/advance", {});
}
export async function compactSession(): Promise<unknown> {
  return rpc("session/compact", {});
}
export async function commandExec(command: string): Promise<unknown> {
  return rpc("command/exec", { command });
}

export function onEvent(cb: (e: WireEvent) => void): Promise<UnlistenFn> {
  return listen<WireEvent>("neo-event", (ev) => cb(ev.payload));
}

export function onStderr(cb: (line: string) => void): Promise<UnlistenFn> {
  return listen<string>("neo-stderr", (ev) => cb(ev.payload));
}

export function onExit(cb: () => void): Promise<UnlistenFn> {
  return listen("neo-exit", () => cb());
}

/** 把 app-server event 通知拆成 { seq, kind, payload } */
export function unpackEvent(e: WireEvent): {
  seq?: number;
  kind: string;
  payload: Record<string, unknown>;
} {
  const p = e.params ?? {};
  return {
    seq: p.seq,
    kind: p.kind ?? "",
    payload: p.payload ?? {},
  };
}

export type { ApprovalState };
