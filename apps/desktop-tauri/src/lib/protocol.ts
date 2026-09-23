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
  /** SessionInfo.wire：changes = [add, del] | null；records / bytes 可选 */
  changes?: [number, number] | null;
  records?: number;
  bytes?: number;
  /** 宽松：部分装配仍写 additions/deletions */
  additions?: number;
  deletions?: number;
};

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
