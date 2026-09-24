/**
 * 线协议类型 —— 与 crates/neo-host-appserver/schema/neo-appserver.ts 同源语义。
 * P0 手写子集；契约全量以 schema 产物为准（门禁逐字节比对那份）。
 */

export const PROTOCOL_VERSION = 1;

export type Decision = "allow" | "allow_always" | "deny";

export type WireEvent = {
  jsonrpc?: string;
  method?: string;
  params?: {
    seq?: number;
    kind?: string;
    payload?: Record<string, unknown>;
  };
};

export type ThreadSummary = {
  id: string;
  title?: string | null;
  has_title?: boolean;
  state?: string;
  /** 文件 mtime（毫秒，Unix epoch）—— 列表已按最近修改排序 */
  updated_ms?: number;
  records?: number;
  bytes?: number;
  changes?: [number, number] | null;
  additions?: number;
  deletions?: number;
  /** Codex thread/archive 标记（sidecar op，列表回读） */
  archived?: boolean;
};

/** Codex threadSection/* */
export type ThreadSection = {
  sectionId: string;
  name: string;
  appearance?: { color?: string | null; icon?: string | null } | null;
  order?: number;
  threadIds?: string[];
};

export type SkillInfo = { name: string; description?: string };

export type PluginInfo = {
  id?: string;
  name?: string;
  version?: string;
  installed?: boolean;
  marketplace?: string;
  path?: string;
  skills?: string[];
};

export type MarketplaceInfo = { name: string; source: string };

export type FileChange = { path: string; additions: number; deletions: number };

export type GoalSubtask = {
  id: number;
  title: string;
  phase?: string;
  retries?: number;
};

export type GoalSnapshot = {
  goal_id: string;
  goal: string;
  paused: boolean;
  stopped: string | null;
  subtasks: GoalSubtask[];
  iterations: number;
  consecutive_failures: number;
  turns_remaining: number;
  budget_used: number;
};

export type TurnSummaryItem = {
  type: "summary";
  input_tokens: number;
  output_tokens: number;
  ms: number;
};

export type InitializeResult = {
  protocol_version?: number;
  methods?: string[];
  server?: { name?: string; version?: string };
  host?: { id?: string; capabilities?: Record<string, unknown> };
};

export type FactItem = {
  kind?: string;
  text?: string;
  [k: string]: unknown;
};

export type HistoryResult = {
  id?: string;
  items?: FactItem[];
  events_replayed?: number;
};

export type ModelsResult = {
  current?: string;
  models?: {
    name: string;
    description?: string;
    context_limit?: number;
    production?: boolean;
    current?: boolean;
  }[];
};

export type ApprovalState = {
  id: string;
  detail: string;
  kind: string;
};

/** 内核 `UserInputRequest` 挂起（request_user_input 反向通道）。 */
export type UserInputState = {
  id: string;
  prompt: string;
};

export type UiItem =
  | { type: "user"; text: string }
  | { type: "assistant"; text: string }
  | { type: "reasoning"; text: string }
  | {
      type: "tool";
      id: string;
      name: string;
      args?: unknown;
      status: "running" | "ok" | "fail" | "approval";
      exitCode?: number;
      stdout?: string;
      stderr?: string;
      truncated?: boolean;
      detail?: string;
      approvalKind?: string;
    }
  | { type: "error"; message: string }
  | { type: "patch"; path: string; diff: string }
  | { type: "turn"; id: string }
  | TurnSummaryItem
  | { type: "files"; files: FileChange[] }
  | { type: "status"; message: string };
