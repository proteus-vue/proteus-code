import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ApprovalState,
  Decision,
  InitializeResult,
  ModelsResult,
  PluginInfo,
  SkillInfo,
  ThreadSection,
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

/** 系统默认浏览器打开 http(s)/file */
export async function openUrl(url: string): Promise<void> {
  return invoke("open_url", { url });
}

/** 抓取 http(s) 页面 HTML（点选前把跨域页变成 srcdoc） */
export async function fetchUrl(url: string): Promise<string> {
  return invoke<string>("fetch_url", { url });
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

/** 应答 `user_input_request`（Codex item/tool/requestUserInput 同形）。 */
export async function respondUserInput(
  id: string,
  response: string,
): Promise<unknown> {
  return rpc("user_input/respond", { id, response });
}

/** 当前目标完整快照（Codex `thread/goal/get`）。无目标时 goal 为 null。 */
export async function goalGet(): Promise<{
  goal?: import("./protocol").GoalSnapshot | null;
}> {
  return rpc("thread/goal/get", {});
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

/** 启动持活 PTY（Codex unified_exec session 模式）。 */
export async function commandExecSession(
  command: string,
  cols = 80,
  rows = 24,
): Promise<{
  session_id?: string;
  running?: boolean;
  output?: string;
  exit_code?: number | null;
  mode?: string;
}> {
  return rpc("command/exec", { command, session: true, cols, rows });
}

export async function execWrite(sessionId: string, data: string): Promise<unknown> {
  return rpc("command/exec/write", { session_id: sessionId, data });
}

export async function execResize(
  sessionId: string,
  cols: number,
  rows: number,
): Promise<unknown> {
  return rpc("command/exec/resize", { session_id: sessionId, cols, rows });
}

export async function execTerminate(sessionId: string): Promise<unknown> {
  return rpc("command/exec/terminate", { session_id: sessionId });
}

/** 协议 fs/*：目录一层 */
export async function fsReadDirectory(path: string): Promise<{
  path?: string;
  entries?: string[];
}> {
  return rpc("fs/readDirectory", { path });
}

export async function fsReadFile(path: string): Promise<{
  path?: string;
  content?: string;
  bytes?: number;
  truncated?: boolean;
}> {
  return rpc("fs/readFile", { path });
}

export async function fsGetMetadata(path: string): Promise<{
  path?: string;
  is_dir?: boolean;
  is_file?: boolean;
  bytes?: number;
}> {
  return rpc("fs/getMetadata", { path });
}

export async function fsWriteFile(path: string, data: string): Promise<unknown> {
  return rpc("fs/writeFile", { path, data });
}

export async function fsCopy(from: string, to: string): Promise<unknown> {
  return rpc("fs/copy", { from, to });
}

export async function fsCreateDirectory(path: string): Promise<unknown> {
  return rpc("fs/createDirectory", { path });
}

export async function fsRemove(path: string): Promise<unknown> {
  return rpc("fs/remove", { path });
}

/** 会话附件侧车 */
export async function threadAttachmentList(id?: string): Promise<{
  id?: string;
  attachments?: unknown[];
}> {
  return rpc("thread/attachment/list", id ? { id } : {});
}

export async function threadAttachmentAdd(
  attachmentType: string,
  identityKey: string,
  payload: unknown,
  id?: string,
): Promise<unknown> {
  return rpc("thread/attachment/add", {
    attachmentType,
    identityKey,
    payload,
    ...(id ? { id } : {}),
  });
}

export async function threadAttachmentRemove(
  attachmentType: string,
  identityKey: string,
  id?: string,
): Promise<unknown> {
  return rpc("thread/attachment/remove", {
    attachmentType,
    identityKey,
    ...(id ? { id } : {}),
  });
}

/** 会话 git 元数据（append-only op） */
export async function threadMetadataUpdate(
  id: string,
  meta: { branch?: string; sha?: string; originUrl?: string },
): Promise<unknown> {
  return rpc("thread/metadata/update", { id, ...meta });
}

export async function configMcpServerReload(): Promise<{
  reloaded?: boolean;
  servers?: string[];
  note?: string;
}> {
  return rpc("config/mcpServer/reload", {});
}

export async function modelProviderCapabilities(): Promise<{
  current?: string;
  models?: unknown[];
}> {
  return rpc("modelProvider/capabilities/read", {});
}

export async function pluginSkillRead(
  plugin: string,
  skill: string,
): Promise<{ content?: string; path?: string; skillName?: string }> {
  return rpc("plugin/skill/read", { pluginName: plugin, skillName: skill });
}

export type MigrationItem = {
  itemType: string;
  description: string;
  cwd?: string;
};

export async function externalAgentDetect(opts?: {
  cwds?: string[];
  includeHome?: boolean;
}): Promise<{ migrationItems?: MigrationItem[]; migrationSource?: string; note?: string }> {
  return rpc("externalAgentConfig/detect", {
    cwds: opts?.cwds,
    includeHome: opts?.includeHome ?? false,
  });
}

export async function externalAgentImport(
  migrationItems: MigrationItem[],
): Promise<{
  imported?: number;
  skipped?: number;
  failed?: number;
  details?: unknown[];
}> {
  return rpc("externalAgentConfig/import", { migrationItems });
}

/** 运行中转向当前轮（Codex `turn/steer`）。 */
export async function steerTurn(text: string): Promise<unknown> {
  return rpc("turn/steer", { text });
}

/** 归档 / 取消归档会话（Codex `thread/archive` / `unarchive`）。 */
export async function archiveThread(id: string, archived: boolean): Promise<unknown> {
  return rpc(archived ? "thread/archive" : "thread/unarchive", { id });
}

/** 线分区列表（Codex `threadSection/list`）。 */
export async function listSections(): Promise<ThreadSection[]> {
  const r = await rpc<{ sections?: ThreadSection[] }>("threadSection/list", {});
  return r.sections ?? [];
}

export async function createSection(name: string): Promise<ThreadSection> {
  return rpc<ThreadSection>("threadSection/create", { name });
}

export async function renameSection(sectionId: string, name: string): Promise<unknown> {
  return rpc("threadSection/update", { sectionId, name });
}

export async function deleteSection(sectionId: string): Promise<unknown> {
  return rpc("threadSection/delete", { sectionId });
}

/** 把线程移入分区；sectionId=null 移出。 */
export async function moveThreadToSection(
  threadId: string,
  sectionId: string | null,
): Promise<unknown> {
  return rpc("thread/section/move", { threadId, sectionId });
}

export async function listSkills(): Promise<{ skills?: SkillInfo[] }> {
  return rpc("skills/list", {});
}

export async function configRead(): Promise<Record<string, unknown>> {
  return rpc("config/read", {});
}

export async function pluginList(): Promise<{
  plugins?: PluginInfo[];
  marketplaces?: { name: string; source: string }[];
}> {
  return rpc("plugin/list", {});
}

export async function pluginInstall(name: string): Promise<unknown> {
  return rpc("plugin/install", { pluginName: name });
}

export async function pluginUninstall(id: string): Promise<unknown> {
  return rpc("plugin/uninstall", { pluginId: id });
}

export async function marketplaceAdd(name: string, source: string): Promise<unknown> {
  return rpc("marketplace/add", { name, source });
}

export async function marketplaceRemove(name: string): Promise<unknown> {
  return rpc("marketplace/remove", { marketplaceName: name });
}

/** 模糊文件搜索（有界）。 */
export async function fuzzyFileSearch(
  query: string,
  roots: string[],
): Promise<{ matches?: { path: string; score?: number }[]; truncated?: boolean }> {
  return rpc("fuzzyFileSearch", { query, roots });
}

/** 导出会话（markdown / json）。 */
export async function threadExport(
  format: "markdown" | "json" = "markdown",
  id?: string,
): Promise<{ content?: string; format?: string; id?: string; items?: unknown }> {
  return rpc("thread/export", id ? { id, format } : { format });
}

/** 按 beforeTurnId 回退（Codex `thread/revert`）。 */
export async function threadRevert(beforeTurnId: string): Promise<unknown> {
  return rpc("thread/revert", { beforeTurnId });
}

/** 向历史注入用户可见文本（不驱动模型）。 */
export async function threadInjectItems(text: string): Promise<unknown> {
  return rpc("thread/inject_items", { text });
}

/** 事实投影分页（与 history 同源）。 */
export async function threadItemsList(
  limit?: number,
  id?: string,
): Promise<{ items?: unknown[]; id?: string }> {
  return rpc("thread/items/list", {
    ...(id ? { id } : {}),
    ...(limit != null ? { limit } : {}),
  });
}

/** 用户轮摘要。 */
export async function threadTurnsList(
  limit?: number,
  id?: string,
): Promise<{ turns?: unknown[]; id?: string }> {
  return rpc("thread/turns/list", {
    ...(id ? { id } : {}),
    ...(limit != null ? { limit } : {}),
  });
}

/** 仅执行本步剩余审批（Codex `approval/respondStep`）。 */
export async function respondApprovalStep(
  id: string,
  decision: Decision,
  reason?: string | null,
): Promise<unknown> {
  return rpc("approval/respondStep", {
    id,
    decision,
    reason: reason ?? null,
  });
}

export async function hooksList(): Promise<{ hooks?: unknown[] }> {
  return rpc("hooks/list", {});
}

export async function mcpServerStatusList(): Promise<{
  servers?: { name?: string; status?: string; transport?: string }[];
}> {
  return rpc("mcpServerStatus/list", {});
}

export async function permissionProfileList(): Promise<{
  profiles?: { id?: string; label?: string; current?: boolean }[];
}> {
  return rpc("permissionProfile/list", {});
}

export async function configRequirementsRead(): Promise<{
  requirements?: { key?: string; required?: boolean; reason?: string }[];
  configFile?: string;
}> {
  return rpc("configRequirements/read", {});
}

/** 写用户 config.json（乐观 _version）。 */
export async function configValueWrite(
  keyPath: string,
  value: unknown,
  mergeStrategy: "replace" | "upsert" = "replace",
): Promise<unknown> {
  return rpc("config/value/write", { keyPath, value, mergeStrategy });
}

/** 启停技能（name 或 path 选择器）。 */
export async function skillsConfigWrite(
  enabled: boolean,
  name?: string,
  path?: string,
): Promise<unknown> {
  return rpc("skills/config/write", {
    enabled,
    ...(name ? { name } : {}),
    ...(path ? { path } : {}),
  });
}

export async function skillsExtraRootsSet(extraRoots: string[]): Promise<unknown> {
  return rpc("skills/extraRoots/set", { extraRoots });
}

export async function experimentalFeatureList(): Promise<{
  features?: { name?: string; enabled?: boolean }[];
}> {
  return rpc("experimentalFeature/list", {});
}

export async function experimentalSet(
  enablement: Record<string, boolean>,
): Promise<unknown> {
  return rpc("experimentalFeature/enablement/set", { enablement });
}

export async function marketplaceUpgrade(name?: string): Promise<unknown> {
  return rpc("marketplace/upgrade", name ? { marketplaceName: name } : {});
}

export async function pluginInstalled(): Promise<{ plugins?: PluginInfo[] }> {
  return rpc("plugin/installed", {});
}

export async function pluginReconcile(): Promise<{
  alive?: number;
  removed?: string[];
}> {
  return rpc("plugin/reconcile", {});
}

export async function pluginRead(name: string): Promise<PluginInfo> {
  return rpc("plugin/read", { pluginName: name });
}

/** 启动审查（返回 prompt，调用方再 turn/start）。 */
export async function reviewStart(
  threadId: string,
  target: { type: string; branch?: string },
  delivery?: "inline" | "detached",
): Promise<{
  started?: boolean;
  prompt?: string;
  delivery?: string;
  drives?: boolean;
  deprecationNotice?: string;
}> {
  return rpc("review/start", {
    threadId,
    target,
    ...(delivery ? { delivery } : {}),
  });
}

/** 本地反馈收据（无远端上传）。 */
export async function feedbackUpload(
  classification: string,
  reason?: string,
): Promise<{ uploaded?: boolean; localPath?: string; note?: string }> {
  return rpc("feedback/upload", {
    classification,
    ...(reason ? { reason } : {}),
  });
}

/** 订阅工作区变化 → fs/changed 事件。 */
export async function fsWatch(path: string, watchId: string): Promise<unknown> {
  return rpc("fs/watch", { path, watchId });
}

export async function fsUnwatch(watchId: string): Promise<unknown> {
  return rpc("fs/unwatch", { watchId });
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
