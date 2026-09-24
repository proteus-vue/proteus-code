import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  ApprovalState,
  Decision,
  FileChange,
  GoalSnapshot,
  InitializeResult,
  ThreadSummary,
  UiItem,
  UserInputState,
  WireEvent,
} from "./lib/protocol";
import { renderMarkdown } from "./lib/markdown";
import { CommandPalette, type CommandItem } from "./components/CommandPalette";
import { DiffView } from "./components/DiffView";
import { DiffModal } from "./components/DiffModal";
import { ProgressFloat } from "./components/ProgressFloat";
import {
  FileTree,
  formatFileRef,
  readWorkspaceFile,
  type FilePreview,
} from "./components/FileTree";
import { BrowserPane, type WebElementPick } from "./components/BrowserPane";
import {
  ThinkingBlock,
  ToolGroup,
  groupTools,
} from "./components/TranscriptParts";
import {
  archiveThread,
  commandExec,
  compactSession,
  configureSession,
  createSection,
  createThread,
  type ExecMode,
  feedbackUpload,
  forkThread,
  fsUnwatch,
  fsWatch,
  gitInfo,
  goalClear,
  goalGet,
  goalPause,
  goalResume,
  goalSet,
  interruptTurn,
  listModels,
  listSections,
  listThreads,
  onEvent,
  onExit,
  onStderr,
  respondApproval,
  respondApprovalStep,
  respondUserInput,
  resumeThread,
  rewindTurns,
  reviewStart,
  startServer,
  startTurn,
  steerTurn,
  stopServer,
  threadExport,
  threadInjectItems,
  threadRevert,
  unpackEvent,
} from "./lib/rpc";
import type { ThreadSection } from "./lib/protocol";
import { listWorkspace } from "./components/FileTree";
import { Sidebar } from "./components/Sidebar";
import { SettingsModal } from "./components/SettingsModal";
import { ProjectPicker } from "./components/ProjectPicker";
import { WorkbenchShell, type WorkbenchId } from "./components/WorkbenchShell";
import { Icon } from "./components/Icon";
import { Select } from "./components/Select";
import {
  deleteThread,
  listTools,
  noProjectDir,
  renameThread,
  type ToolInfo,
} from "./lib/rpc";
import {
  dueNow,
  loadAutomations,
  nextDue,
  saveAutomations,
  type Automation,
} from "./lib/automations";
import {
  loadRecentWorkspaces,
  touchWorkspace,
  type RecentWorkspace,
} from "./lib/workspaces";
import "./styles/tokens.css";
import "./styles/app.css";
import "./styles/layout.css";

type Status = "boot" | "ready" | "busy" | "error";
type WB = WorkbenchId;
type SplitEdge = "sidebar" | "panel";

const EXEC_MODES: { id: ExecMode; label: string }[] = [
  { id: "plan", label: "Plan" },
  { id: "confirm_before", label: "确认" },
  { id: "default", label: "Default" },
  { id: "auto_edit", label: "自动编辑" },
  { id: "full_access", label: "完全访问" },
];

/** IA-11 分栏范围（PRODUCT-IA §2：sidebar 256–275 默认；panel ≥320） */
const SIDEBAR_MIN = 200;
const SIDEBAR_MAX = 420;
const SIDEBAR_DEFAULT = 264;
const PANEL_MIN = 320;
const PANEL_MAX = 720;
const PANEL_DEFAULT = 380;

function clamp(n: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, n));
}

function readStoredPx(key: string, fallback: number, min: number, max: number): number {
  const raw = Number(localStorage.getItem(key));
  if (!Number.isFinite(raw) || raw <= 0) return fallback;
  return clamp(Math.round(raw), min, max);
}

/** IA-10 壳侧斜杠命令 —— 只列**真有动作**的（对齐 TUI「不实现不列出」） */
const SLASH_COMMANDS: {
  id: string;
  name: string;
  aliases?: string[];
  desc: string;
}[] = [
  { id: "compact", name: "compact", aliases: ["summarize"], desc: "压缩上下文" },
  { id: "new", name: "new", aliases: ["clear"], desc: "新建会话" },
  { id: "fork", name: "fork", desc: "分叉当前会话" },
  { id: "undo", name: "undo", aliases: ["rewind"], desc: "回退一轮" },
  { id: "export", name: "export", aliases: ["dump"], desc: "导出对话 markdown" },
  { id: "inject", name: "inject", desc: "注入文本不驱动模型" },
  { id: "revert", name: "revert", desc: "回退到指定 turn（thread/revert）" },
  { id: "review", name: "review", desc: "审查未提交改动（review/start）" },
  { id: "feedback", name: "feedback", desc: "本地反馈收据" },
  { id: "settings", name: "settings", aliases: ["config"], desc: "打开设置 ⌘," },
  { id: "theme", name: "theme", aliases: ["themes"], desc: "切换浅色/深色" },
  { id: "sidebar", name: "sidebar", desc: "切换侧栏 ⌘B" },
  { id: "panel", name: "panel", desc: "切换右栏 ⌘J" },
  { id: "mode", name: "mode", desc: "循环执行模式 ⇧Tab" },
  { id: "models", name: "models", aliases: ["model"], desc: "打开模型设置" },
  { id: "help", name: "help", desc: "命令面板 ⌘K" },
];

function matchSlash(q: string) {
  const s = q.toLowerCase();
  return SLASH_COMMANDS.filter(
    (c) =>
      c.name.startsWith(s) ||
      (c.aliases ?? []).some((a) => a.startsWith(s)),
  );
}


/** unified diff 行数统计（排除元数据行） */
function formatWorkDur(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const m = Math.floor(s / 60);
  const r = s % 60;
  if (m <= 0) return `${r} 秒`;
  return `${m} 分 ${r} 秒`;
}

function diffStats(diff: string): { a: number; d: number } {
  let a = 0;
  let d = 0;
  for (const line of diff.split(/\r?\n/)) {
    if (
      line.startsWith("+++") ||
      line.startsWith("---") ||
      line.startsWith("diff ") ||
      line.startsWith("index ") ||
      line.startsWith("@@")
    ) {
      continue;
    }
    if (line.startsWith("+")) a++;
    else if (line.startsWith("-")) d++;
  }
  return { a, d };
}

export default function App() {
  const [status, setStatus] = useState<Status>("boot");
  const [statusMsg, setStatusMsg] = useState("正在连接 app-server…");
  const [init, setInit] = useState<InitializeResult | null>(null);
  const [threads, setThreads] = useState<ThreadSummary[]>([]);
  const [activeThread, setActiveThread] = useState<string | null>(null);
  /** IA-49：threadSection/* 分区 */
  const [sections, setSections] = useState<ThreadSection[]>([]);
  /** 文件树自动刷新键（files_changed / fs_changed 递增） */
  const [fileTreeKey, setFileTreeKey] = useState(0);
  const [model, setModel] = useState<string>("—");
  const [modelsList, setModelsList] = useState<
    { name: string; description?: string; production?: boolean }[]
  >([]);
  const [branch, setBranch] = useState<string>("—");
  const [workspaceRoot, setWorkspaceRoot] = useState<string>("");
  /** IA-22：workspace=绑项目；none=不在项目中工作（~/.neo/no-project） */
  const [projectMode, setProjectMode] = useState<"workspace" | "none">(() => {
    const m = localStorage.getItem("neo-ia-project-mode");
    if (m === "none" || m === "workspace") return m;
    return localStorage.getItem("neo-ia-workspace") || import.meta.env.VITE_NEO_WORKSPACE
      ? "workspace"
      : "none";
  });
  /** IA-20 最近工作区 + 切换中（忽略 onExit 误报） */
  const [recentWs, setRecentWs] = useState<RecentWorkspace[]>(() =>
    loadRecentWorkspaces(),
  );
  const switchingWs = useRef(false);
  const [items, setItems] = useState<UiItem[]>([]);
  const [approval, setApproval] = useState<ApprovalState | null>(null);
  /** request_user_input 挂起（user_input_request → user_input/respond） */
  const [userInput, setUserInput] = useState<UserInputState | null>(null);
  const [userInputText, setUserInputText] = useState("");
  const [input, setInput] = useState("");
  /** IA-35：选中的网页元素附件（chip），不塞进 textarea */
  const [webPicks, setWebPicks] = useState<WebElementPick[]>([]);
  const [stderrLines, setStderrLines] = useState<string[]>([]);
  const [execMode, setExecMode] = useState<ExecMode>("default");
  const [panelOpen, setPanelOpen] = useState(true);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  /** IA-11 分栏宽度（localStorage 持久化） */
  const [sidebarW, setSidebarW] = useState(() =>
    readStoredPx("neo-sidebar-w", SIDEBAR_DEFAULT, SIDEBAR_MIN, SIDEBAR_MAX),
  );
  const [panelW, setPanelW] = useState(() =>
    readStoredPx("neo-panel-w", PANEL_DEFAULT, PANEL_MIN, PANEL_MAX),
  );
  /** 浏览器式标签：已打开的右栏页（可多开，× 关闭） */
  const [openTabs, setOpenTabs] = useState<WB[]>(["review"]);
  const [panelTab, setPanelTab] = useState<WB>("review");
  const [addMenuOpen, setAddMenuOpen] = useState(false);
  const [lastPatch, setLastPatch] = useState<{ path: string; diff: string } | null>(null);
  const [files, setFiles] = useState<FileChange[]>([]);
  const [goal, setGoal] = useState<GoalSnapshot | null>(null);
  const [goalInput, setGoalInput] = useState("");
  const [thinkingSearch, setThinkingSearch] = useState("");
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  /** IA-24：Composer 芯片上的项目下拉（ZCode） */
  const [projPickerOpen, setProjPickerOpen] = useState(false);
  const [projAnchor, setProjAnchor] = useState<{ left: number; top: number } | null>(null);
  const [theme, setTheme] = useState<"light" | "dark">(
    () =>
      (localStorage.getItem("neo-theme") as "light" | "dark" | null) ??
      "light",
  );
  const [termLog, setTermLog] = useState<
    {
      id?: string;
      cmd: string;
      /** null = 进行中（等 tool_call_end） */
      ok: boolean | null;
      stdout?: string;
      stderr?: string;
      truncated?: boolean;
    }[]
  >([]);
  const [termInput, setTermInput] = useState("");
  /** IA-14：tools/list 里的 agent_* 子代理 */
  const [agentTools, setAgentTools] = useState<ToolInfo[]>([]);
  /** IA-14 Automations：壳侧排程（localStorage） */
  const [autos, setAutos] = useState<Automation[]>(() => loadAutomations());
  const [autoTitle, setAutoTitle] = useState("");
  const [autoPrompt, setAutoPrompt] = useState("");
  const [autoEvery, setAutoEvery] = useState(60);
  /** 正在跑的 automation id，防重入 */
  const autoRunning = useRef(false);
  const [diffModal, setDiffModal] = useState<{ path: string; diff: string } | null>(null);
  const [filePreview, setFilePreview] = useState<FilePreview | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [floatCollapsed, setFloatCollapsed] = useState(false);
  const [fileEntries, setFileEntries] = useState<string[]>([]);
  const [atQuery, setAtQuery] = useState<{ prefix: string; q: string } | null>(null);
  const [atIdx, setAtIdx] = useState(0);
  /** IA-10：整行 `/query` 时的斜杠候选下标 */
  const [slashIdx, setSlashIdx] = useState(0);
  const [turnStartedAt, setTurnStartedAt] = useState<number | null>(null);
  const [workTick, setWorkTick] = useState(0);
  useEffect(() => {
    if (status !== "busy" || !turnStartedAt) return;
    const iv = window.setInterval(() => setWorkTick((n) => n + 1), 1000);
    return () => window.clearInterval(iv);
  }, [status, turnStartedAt]);
  void workTick;
  const [lastSummary, setLastSummary] = useState<{
    in: number;
    out: number;
    ms: number;
  } | null>(null);
  const streamRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  /** 斜杠执行器：定义在 onFork/cycleMode 之后，经 ref 注入 send */
  const slashRunnerRef = useRef<(text: string) => Promise<boolean>>(
    async () => false,
  );

  const busy = status === "busy";
  /** 审批 / 问人挂起时锁输入；busy 可 steer，不进 blocked */
  const blocked = !!approval || !!userInput;
  /** 调度器读最新 status/busy，避免闭包过期 */
  const statusRef = useRef(status);
  statusRef.current = status;
  const busyRef = useRef(busy);
  busyRef.current = busy;

  const append = useCallback((item: UiItem) => {
    setItems((prev) => [...prev, item]);
  }, []);

  const mergeAssistantDelta = useCallback((delta: string) => {
    setItems((prev) => {
      const next = [...prev];
      const last = next[next.length - 1];
      if (last && last.type === "assistant") {
        next[next.length - 1] = { ...last, text: last.text + delta };
        return next;
      }
      next.push({ type: "assistant", text: delta });
      return next;
    });
  }, []);

  const mergeReasoningDelta = useCallback((delta: string) => {
    setItems((prev) => {
      const next = [...prev];
      const last = next[next.length - 1];
      if (last && last.type === "reasoning") {
        next[next.length - 1] = { ...last, text: last.text + delta };
        return next;
      }
      next.push({ type: "reasoning", text: delta });
      return next;
    });
  }, []);

  const handleEvent = useCallback(
    (wire: WireEvent) => {
      const { kind, payload } = unpackEvent(wire);
      switch (kind) {
        case "user_submitted":
          append({ type: "user", text: String(payload.text ?? "") });
          setStatus("busy");
          break;
        case "turn_started":
          append({ type: "turn", id: String(payload.turn_id ?? "") });
          setTurnStartedAt(Date.now());
          setStatus("busy");
          break;
        case "agent_message_delta":
          mergeAssistantDelta(String(payload.delta ?? ""));
          break;
        case "agent_message_done": {
          const doneText = typeof payload.text === "string" ? payload.text : "";
          if (doneText) {
            setItems((prev) => {
              const next = [...prev];
              const last = next[next.length - 1];
              if (last && last.type === "assistant" && last.text === "") {
                next[next.length - 1] = { type: "assistant", text: doneText };
              }
              return next;
            });
          }
          break;
        }
        case "reasoning_delta":
          mergeReasoningDelta(String(payload.delta ?? ""));
          break;
        case "tool_call_begin": {
          const evId = String(payload.id ?? "");
          const evName = String(payload.name ?? "tool");
          const args = payload.arguments as { cmd?: unknown } | undefined;
          // 用户命令台：Op::Shell → id shell-* / name bash
          if (evId.startsWith("shell-")) {
            setTermLog((log) => [
              ...log.slice(-200),
              { id: evId, cmd: String(args?.cmd ?? ""), ok: null },
            ]);
          }
          append({
            type: "tool",
            id: evId,
            name: evName,
            args: payload.arguments,
            status: "running",
          });
          break;
        }
        case "tool_call_end": {
          const evId = String(payload.id ?? "");
          const code = Number(payload.exit_code ?? -1);
          if (evId.startsWith("shell-")) {
            setTermLog((log) =>
              log.map((row) =>
                row.id === evId
                  ? {
                      ...row,
                      ok: code === 0,
                      stdout: String(payload.stdout ?? ""),
                      stderr: String(payload.stderr ?? ""),
                      truncated: Boolean(payload.truncated),
                    }
                  : row,
              ),
            );
          }
          setItems((prev) =>
            prev.map((it) => {
              if (it.type === "tool" && it.id === payload.id) {
                return {
                  ...it,
                  status: code === 0 ? "ok" : "fail",
                  exitCode: code,
                  stdout: String(payload.stdout ?? ""),
                  stderr: String(payload.stderr ?? ""),
                  truncated: Boolean(payload.truncated),
                };
              }
              return it;
            }),
          );
          break;
        }
        case "approval_request":
          setApproval({
            id: String(payload.id ?? ""),
            detail: String(payload.detail ?? ""),
            kind: String(payload.kind ?? "write"),
          });
          setItems((prev) =>
            prev.map((it) => {
              if (it.type === "tool" && it.status === "running") {
                return {
                  ...it,
                  status: "approval",
                  detail: String(payload.detail ?? ""),
                  approvalKind: String(payload.kind ?? ""),
                };
              }
              return it;
            }),
          );
          break;
        case "user_input_request":
          setUserInput({
            id: String(payload.id ?? ""),
            prompt: String(payload.prompt ?? ""),
          });
          setUserInputText("");
          setItems((prev) =>
            prev.map((it) => {
              if (it.type === "tool" && it.status === "running") {
                return {
                  ...it,
                  status: "approval",
                  detail: String(payload.prompt ?? ""),
                  approvalKind: "interactive",
                };
              }
              return it;
            }),
          );
          break;
        case "patch_proposed":
          setLastPatch({
            path: String(payload.path ?? ""),
            diff: String(payload.diff ?? ""),
          });
          append({
            type: "patch",
            path: String(payload.path ?? ""),
            diff: String(payload.diff ?? ""),
          });
          break;
        case "files_changed":
        case "file_changed":
        case "fs_changed": {
          if (kind === "files_changed") {
            const fs = (payload.files as FileChange[] | undefined) ?? [];
            setFiles(fs);
            append({ type: "files", files: fs });
          }
          // 文件树跟随磁盘（batch6 fs/watch 或内核 files_changed）
          setFileTreeKey((n) => n + 1);
          break;
        }
        case "goal_updated": {
          const snap = payload.snapshot as GoalSnapshot | undefined;
          if (snap) setGoal(snap);
          break;
        }
        case "goal_cleared":
          setGoal(null);
          break;
        case "goal_progress":
          break;
        case "model_switched":
          setModel(String(payload.model ?? model));
          break;
        case "rewound":
          setApproval(null);
          setUserInput(null);
          setUserInputText("");
          setLastPatch(null);
          setItems((prev) => {
            let lastUser = -1;
            for (let i = prev.length - 1; i >= 0; i--) {
              if (prev[i].type === "user") {
                lastUser = i;
                break;
              }
            }
            return lastUser >= 0 ? prev.slice(0, lastUser) : prev;
          });
          break;
        case "error":
          append({ type: "error", message: String(payload.message ?? "error") });
          break;
        case "turn_complete": {
          const tin = Number(payload.input_tokens ?? 0);
          const tout = Number(payload.output_tokens ?? 0);
          const ms = turnStartedAt ? Date.now() - turnStartedAt : 0;
          setLastSummary({ in: tin, out: tout, ms });
          append({ type: "summary", input_tokens: tin, output_tokens: tout, ms });
          setTurnStartedAt(null);
          setStatus("ready");
          setStatusMsg(`+${tin}/−${tout} tok · ${(ms / 1000).toFixed(1)}s`);
          void refreshThreads();
          break;
        }
        case "session_configured":
          break;
        case "shutdown_complete":
          setStatus("ready");
          break;
        default:
          break;
      }
    },
    // turnStartedAt 放进依赖会因时间戳抖动重建 handler；用 ref 稳定
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [append, mergeAssistantDelta, mergeReasoningDelta, model],
  );

  const turnRef = useRef(turnStartedAt);
  turnRef.current = turnStartedAt;
  // 修正 turn_complete 读 ref —— 上面闭包捕获的是旧值，单独用 ref 版
  const handleRef = useRef(handleEvent);
  // 用包装：turn_complete 时读 turnRef
  const handleStable = useCallback(
    (wire: WireEvent) => {
      const { kind, payload } = unpackEvent(wire);
      if (kind === "turn_complete") {
        const tin = Number(payload.input_tokens ?? 0);
        const tout = Number(payload.output_tokens ?? 0);
        const start = turnRef.current;
        const ms = start ? Date.now() - start : 0;
        setLastSummary({ in: tin, out: tout, ms });
        setItems((prev) => [
          ...prev,
          { type: "summary", input_tokens: tin, output_tokens: tout, ms },
        ]);
        setTurnStartedAt(null);
        setStatus("ready");
        setStatusMsg(`+${tin}/−${tout} tok · ${(ms / 1000).toFixed(1)}s`);
        void refreshThreadsRef.current?.();
        return;
      }
      if (kind === "turn_started") {
        setTurnStartedAt(Date.now());
        turnRef.current = Date.now();
      }
      handleRef.current(wire);
    },
    [],
  );
  handleRef.current = handleEvent;

  const refreshThreads = useCallback(async () => {
    try {
      setThreads(await listThreads());
    } catch {
      /* ignore */
    }
  }, []);
  const refreshThreadsRef = useRef(refreshThreads);
  refreshThreadsRef.current = refreshThreads;

  /** IA-49：线分区列表 */
  const refreshSections = useCallback(async () => {
    try {
      setSections(await listSections());
    } catch {
      setSections([]);
    }
  }, []);

  /** Codex thread/goal/get：boot / 切会话后刷 Goal 面板。 */
  const refreshGoal = useCallback(async () => {
    try {
      const r = await goalGet();
      setGoal(r.goal ?? null);
    } catch {
      /* 旧方法表无此法时静默 */
    }
  }, []);

  const boot = useCallback(
    async (override?: { workspace?: string; mode?: "workspace" | "none" }) => {
      setStatus("boot");
      setStatusMsg("正在连接 app-server…");
      try {
        let mode: "workspace" | "none" =
          override?.mode ??
          ((localStorage.getItem("neo-ia-project-mode") as "workspace" | "none" | null) ??
            "none");
        let ws =
          (override?.workspace ?? localStorage.getItem("neo-ia-workspace"))?.trim() ||
          import.meta.env.VITE_NEO_WORKSPACE?.trim() ||
          "";
        if (mode === "workspace" && !ws) {
          // 没选过项目 → 落到「不在项目中」，禁止拿进程 cwd 冒充项目
          mode = "none";
        }
        if (mode === "none") {
          ws = (await noProjectDir()).trim();
        }

        const info = await startServer({
          provider: "mock",
          workspace: ws || undefined,
        });
        localStorage.setItem("neo-ia-project-mode", mode);
        if (mode === "workspace" && ws) {
          localStorage.setItem("neo-ia-workspace", ws);
          setRecentWs(touchWorkspace(ws));
        }
        setProjectMode(mode);
        setInit(info);
        const m = await listModels().catch(() => null);
        if (m?.current) setModel(m.current);
        if (m?.models?.length) {
          setModelsList(
            m.models.map((x) => ({
              name: x.name,
              description: x.description,
              production: x.production,
            })),
          );
        }
        void listTools()
          .then((r) =>
            setAgentTools(
              (r.tools ?? []).filter((t) => t.name.startsWith("agent_")),
            ),
          )
          .catch(() => setAgentTools([]));
        const g = await gitInfo().catch(() => null);
        if (g && typeof g.branch === "string" && g.branch) setBranch(g.branch);
        else setBranch("—");
        // 实际传给 app-server 的目录（none = no-project）
        const root =
          mode === "workspace"
            ? (typeof g?.root === "string" && g.root ? g.root : ws)
            : ws;
        setWorkspaceRoot(root);
        if (root) {
          void listWorkspace(root)
            .then((r) =>
              setFileEntries(r.entries.filter((e) => !e.endsWith("/"))),
            )
            .catch(() => setFileEntries([]));
          // fs/watch：工作区变化 → fs_changed → 文件树 refreshKey
          void fsUnwatch("ws-root").catch(() => undefined);
          void fsWatch(root, "ws-root").catch(() => undefined);
        } else {
          void fsUnwatch("ws-root").catch(() => undefined);
        }
        await refreshThreads();
        void refreshGoal();
        void refreshSections();
        setStatus("ready");
        setStatusMsg(
          mode === "none"
            ? "已连接 · 不在项目中工作"
            : `已连接 · ${ws.split("/").filter(Boolean).pop() ?? ws}`,
        );
      } catch (e) {
        setStatus("error");
        setStatusMsg(String(e));
      }
    },
    [refreshGoal, refreshSections, refreshThreads],
  );

  const resetSessionUi = useCallback(() => {
    setItems([]);
    setApproval(null);
    setUserInput(null);
    setUserInputText("");
    setFiles([]);
    setLastPatch(null);
    setActiveThread(null);
    setThreads([]);
    setWorkspaceRoot("");
    setBranch("—");
    setFileEntries([]);
    setFilePreview(null);
    setInput("");
    setWebPicks([]);
  }, []);

  /** 真切换到某个项目目录 */
  const switchWorkspace = useCallback(
    async (path: string) => {
      const clean = path.replace(/\/+$/, "");
      if (!clean) return;
      if (projectMode === "workspace" && clean === workspaceRoot.replace(/\/+$/, "")) {
        return;
      }
      switchingWs.current = true;
      setStatus("boot");
      setStatusMsg(`切换到 ${clean.split("/").filter(Boolean).pop() ?? clean}…`);
      try {
        resetSessionUi();
        await boot({ workspace: clean, mode: "workspace" });
      } finally {
        switchingWs.current = false;
      }
    },
    [boot, projectMode, resetSessionUi, workspaceRoot],
  );

  /** 不在项目中工作：切到 ~/.neo/no-project */
  const switchNoProject = useCallback(async () => {
    if (projectMode === "none") return;
    switchingWs.current = true;
    setStatus("boot");
    setStatusMsg("切换到不在项目中工作…");
    try {
      resetSessionUi();
      await boot({ mode: "none" });
    } finally {
      switchingWs.current = false;
    }
  }, [boot, projectMode, resetSessionUi]);

  useEffect(() => {
    const unsubs: Array<() => void> = [];
    void (async () => {
      unsubs.push(
        await onEvent((e) => handleStable(e)),
        await onStderr((line) =>
          setStderrLines((ls) => [...ls.slice(-40), line]),
        ),
        await onExit(() => {
          // 切换 workspace 时 start_app_server 会先杀旧进程 —— 不当作崩溃
          if (switchingWs.current) return;
          setStatus("error");
          setStatusMsg("app-server 进程已退出");
        }),
      );
    })();
    void boot();
    return () => {
      unsubs.forEach((u) => u());
      void stopServer().catch(() => undefined);
    };
  }, [boot, handleStable]);

  useEffect(() => {
    const el = streamRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [items, approval]);

  const formatWebPick = (el: WebElementPick): string => {
    const parts: string[] = [`【网页元素】 ${el.url}`, `selector: \`${el.selector}\``];
    if (el.size) parts.push(`size: ${el.size}`);
    if (el.color) parts.push(`color: ${el.color}`);
    if (el.font) parts.push(`font: ${el.font}`);
    if (el.text) parts.push(`\ntext:\n${el.text}`);
    if (el.html) parts.push(`\nhtml:\n${el.html}`);
    return parts.join("\n");
  };

  const send = useCallback(async () => {
    const text = input.trim();
    // 审批 / 问人挂起时锁定；生成中允许 **转向**（turn/steer），不新开轮
    if (approval || userInput) return;
    if ((!text && webPicks.length === 0)) return;
    // 附件在发送时拼进消息，不占输入框
    const withAtt =
      webPicks.length > 0
        ? [text, ...webPicks.map(formatWebPick)].filter(Boolean).join("\n\n")
        : text;
    if (!withAtt) return;
    if (await slashRunnerRef.current(text || withAtt)) {
      setInput("");
      setWebPicks([]);
      return;
    }
    setInput("");
    setWebPicks([]);
    try {
      if (busy) {
        await steerTurn(withAtt);
        append({ type: "user", text: withAtt });
        append({ type: "status", message: "已转向当前轮（turn/steer）" });
        return;
      }
      await startTurn(withAtt);
    } catch (e) {
      append({ type: "error", message: String(e) });
      if (!busy) setStatus("error");
    }
  }, [append, approval, busy, input, userInput, webPicks]);

  const stop = useCallback(async () => {
    try {
      await interruptTurn();
    } catch (e) {
      append({ type: "error", message: String(e) });
    }
  }, [append]);

  const onApproval = useCallback(
    async (decision: Decision, step = false) => {
      if (!approval) return;
      const id = approval.id;
      setApproval(null);
      try {
        if (step) await respondApprovalStep(id, decision, null);
        else await respondApproval(id, decision, null);
      } catch (e) {
        append({ type: "error", message: String(e) });
      }
    },
    [approval, append],
  );

  const onUserInputSubmit = useCallback(async () => {
    if (!userInput) return;
    const id = userInput.id;
    const response = userInputText;
    setUserInput(null);
    setUserInputText("");
    try {
      await respondUserInput(id, response);
    } catch (e) {
      append({ type: "error", message: String(e) });
    }
  }, [append, userInput, userInputText]);

  /** Codex thread/goal/get：boot / 切会话后刷 Goal 面板。 */
  // refreshGoal 已上移至 boot 之前（避免 TDZ）

  const onModeChange = useCallback(
    async (m: ExecMode) => {
      setExecMode(m);
      try {
        await configureSession({ exec_mode: m });
        setStatusMsg(`模式 ${m}`);
      } catch (e) {
        append({ type: "error", message: String(e) });
      }
    },
    [append],
  );

  const onNewThread = useCallback(async () => {
    try {
      const id = await createThread();
      setActiveThread(id);
      setItems([]);
      setInput("");
      setApproval(null);
      setUserInput(null);
      setUserInputText("");
      setLastPatch(null);
      setGoal(null);
      setFiles([]);
      await refreshThreads();
      requestAnimationFrame(() => inputRef.current?.focus());
    } catch (e) {
      append({ type: "error", message: String(e) });
    }
  }, [append, refreshThreads]);

  const onResume = useCallback(
    async (id: string) => {
      try {
        setActiveThread(id);
        setItems([]);
        setApproval(null);
        setUserInput(null);
        setUserInputText("");
        setLastPatch(null);
        await resumeThread(id);
        await refreshThreads();
        await refreshGoal();
        setStatusMsg(`已切换 ${id}`);
      } catch (e) {
        append({ type: "error", message: String(e) });
      }
    },
    [append, refreshGoal, refreshThreads],
  );

  const onSetGoal = useCallback(async () => {
    const g = goalInput.trim();
    if (!g) return;
    try {
      await goalSet(g);
      setGoalInput("");
    } catch (e) {
      append({ type: "error", message: String(e) });
    }
  }, [append, goalInput]);

  const onModelChange = useCallback(
    async (name: string) => {
      try {
        await configureSession({ model: name });
        setModel(name);
        setStatusMsg(`模型 → ${name}`);
      } catch (e) {
        append({ type: "error", message: String(e) });
      }
    },
    [append],
  );

  const cycleMode = useCallback(() => {
    const i = EXEC_MODES.findIndex((m) => m.id === execMode);
    void onModeChange(EXEC_MODES[(i + 1) % EXEC_MODES.length].id);
  }, [execMode, onModeChange]);

  const onFork = useCallback(async () => {
    if (busy) return;
    try {
      await forkThread();
      append({ type: "status", message: "已分叉当前会话（session/fork）" });
      await refreshThreads();
    } catch (e) {
      append({ type: "error", message: String(e) });
    }
  }, [append, busy, refreshThreads]);

  /** IA-10：整行 `/cmd` 由壳执行；未命中则交给 turn/start */
  const runSlashLine = useCallback(
    async (text: string): Promise<boolean> => {
      if (!text.startsWith("/")) return false;
      const token = text.slice(1).split(/\s+/)[0]?.toLowerCase() ?? "";
      const hit = SLASH_COMMANDS.find(
        (c) => c.name === token || (c.aliases ?? []).includes(token),
      );
      if (!hit) return false;
      switch (hit.id) {
        case "compact":
          try {
            await compactSession();
            append({ type: "status", message: "已请求上下文压缩" });
          } catch (e) {
            append({ type: "error", message: String(e) });
          }
          break;
        case "new":
          await onNewThread();
          break;
        case "fork":
          await onFork();
          break;
        case "undo":
          try {
            await rewindTurns(1);
            append({ type: "status", message: "已回退一轮" });
          } catch (e) {
            append({ type: "error", message: String(e) });
          }
          break;
        case "export": {
          try {
            const r = await threadExport("markdown");
            const content = r.content ?? "";
            append({
              type: "status",
              message: `已导出 markdown（${content.length} 字）· thread/export`,
            });
            // 内容进状态行上方：用 inject 也不合适；直接塞一条可复制 assistant 摘要
            if (content) {
              append({
                type: "assistant",
                text: "```\n导出预览（前 500 字）：\n" +
                  content.slice(0, 500) +
                  (content.length > 500 ? "\n…" : "") +
                  "\n```",
              });
            }
          } catch (e) {
            append({ type: "error", message: String(e) });
          }
          break;
        }
        case "inject": {
          const payload = text.slice("/inject".length).trim();
          if (!payload) {
            append({ type: "error", message: "用法：/inject 要注入的文本" });
            break;
          }
          try {
            await threadInjectItems(payload);
            append({ type: "status", message: "已注入历史（不驱动模型）" });
          } catch (e) {
            append({ type: "error", message: String(e) });
          }
          break;
        }
        case "revert": {
          const tid = text.slice("/revert".length).trim() || "turn-1";
          try {
            await threadRevert(tid);
            append({ type: "status", message: `已回退到 ${tid}` });
            await refreshGoal();
          } catch (e) {
            append({ type: "error", message: String(e) });
          }
          break;
        }
        case "review": {
          try {
            const threadId = activeThread ?? "";
            const r = await reviewStart(threadId || "appserver", {
              type: "uncommittedChanges",
            });
            if (r.prompt) {
              append({ type: "status", message: "审查 prompt 已就绪，发送中…" });
              await startTurn(r.prompt);
            } else {
              append({ type: "status", message: "review/start 未返回 prompt" });
            }
          } catch (e) {
            append({ type: "error", message: String(e) });
          }
          break;
        }
        case "feedback": {
          try {
            const r = await feedbackUpload("user_note", "slash /feedback");
            append({
              type: "status",
              message: r.localPath
                ? `反馈已本地保存 ${r.localPath}`
                : "反馈已记录",
            });
          } catch (e) {
            append({ type: "error", message: String(e) });
          }
          break;
        }
        case "settings":
        case "models":
          setSettingsOpen(true);
          break;
        case "theme":
          setTheme((v) => (v === "light" ? "dark" : "light"));
          break;
        case "sidebar":
          setSidebarOpen((v) => !v);
          break;
        case "panel":
          setPanelOpen((v) => !v);
          break;
        case "mode":
          cycleMode();
          break;
        case "help":
          setPaletteOpen(true);
          break;
        default:
          return false;
      }
      return true;
    },
    [activeThread, append, cycleMode, onFork, onNewThread, refreshGoal],
  );

  useEffect(() => {
    slashRunnerRef.current = runSlashLine;
  }, [runSlashLine]);

  /** IA-14：跑一条 automation（手动或到点） */
  const runAutomation = useCallback(
    async (id: string) => {
      if (autoRunning.current) return;
      const a = autos.find((x) => x.id === id);
      if (!a || !a.prompt.trim()) return;
      if (statusRef.current !== "ready" && statusRef.current !== "busy") return;
      if (busyRef.current) return;
      autoRunning.current = true;
      const now = Date.now();
      try {
        // 走主对话：结构与用户手输一致（模型可见、可审计）
        await startTurn(
          `[Automations · ${a.title || a.id}] ${a.prompt.trim()}`,
        );
        setAutos((list) => {
          const next = list.map((x) =>
            x.id === id
              ? {
                  ...x,
                  lastRunMs: now,
                  nextRunMs: x.everyMin > 0 ? now + x.everyMin * 60_000 : 0,
                  lastError: undefined,
                }
              : x,
          );
          saveAutomations(next);
          return next;
        });
        append({ type: "status", message: `Automations 触发：${a.title || id}` });
      } catch (e) {
        setAutos((list) => {
          const next = list.map((x) =>
            x.id === id
              ? { ...x, lastRunMs: now, lastError: String(e) }
              : x,
          );
          saveAutomations(next);
          return next;
        });
        append({ type: "error", message: `Automations 失败：${String(e)}` });
      } finally {
        autoRunning.current = false;
      }
    },
    [append, autos],
  );

  // 到点扫描：30s 一次（无固定 sleep；条件在 dueNow + status gate）
  useEffect(() => {
    const tick = () => {
      if (statusRef.current !== "ready" || busyRef.current) return;
      const due = dueNow(autos);
      if (due[0]) void runAutomation(due[0].id);
    };
    const t0 = window.setTimeout(tick, 2000);
    const iv = window.setInterval(tick, 30_000);
    return () => {
      window.clearTimeout(t0);
      window.clearInterval(iv);
    };
  }, [autos, runAutomation]);

  const persistAutos = useCallback((list: Automation[]) => {
    setAutos(list);
    saveAutomations(list);
  }, []);

  const runTerminal = useCallback(
    async (cmd: string) => {
      const c = cmd.trim();
      if (!c) return;
      try {
        // 只清输入；结果行由 shell-* 的 tool_call_begin/end 回填（含 stdout）
        await commandExec(c);
        setTermInput("");
        append({ type: "status", message: `$ ${c}` });
      } catch (e) {
        setTermLog((log) => [
          ...log.slice(-200),
          { cmd: c, ok: false, stderr: String(e) },
        ]);
        append({ type: "error", message: String(e) });
      }
    },
    [append],
  );

  const openWorkbench = useCallback((tab: WB) => {
    setOpenTabs((tabs) => (tabs.includes(tab) ? tabs : [...tabs, tab]));
    setPanelTab(tab);
    setPanelOpen(true);
    setAddMenuOpen(false);
  }, []);

  const closeWorkbenchTab = useCallback((tab: WB) => {
    setOpenTabs((tabs) => {
      const next = tabs.filter((t) => t !== tab);
      if (next.length === 0) setPanelOpen(false);
      setPanelTab((cur) => (cur !== tab ? cur : next[0] ?? cur));
      return next;
    });
    setAddMenuOpen(false);
  }, []);

  // 点菜单外关闭 —— WorkbenchShell 内部自管；此处保留兼容
  useEffect(() => {
    if (!addMenuOpen) return;
    const onDoc = (e: MouseEvent) => {
      const t = e.target as HTMLElement;
      if (!t.closest(".btab-add")) setAddMenuOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [addMenuOpen]);

  const onDeleteSession = useCallback(
    async (id: string) => {
      try {
        let removed = false;
        try {
          removed = await deleteThread(id);
        } catch (e) {
          const msg = String(e);
          // 内核拒删当前会话：先切走（或新建）再删一次
          if (!msg.includes("不能删除当前")) throw e;
          const list = await listThreads().catch(() => threads);
          const other = list.find((t) => t.id !== id);
          if (other) {
            await resumeThread(other.id);
            setActiveThread(other.id);
            setItems([]);
            setApproval(null);
          } else {
            const nid = await createThread();
            setActiveThread(nid);
            setItems([]);
          }
          removed = await deleteThread(id);
        }
        if (activeThread === id) setActiveThread(null);
        // 直接拉列表，避免 refresh 静默吞错导致「删了还在」
        try {
          setThreads(await listThreads());
        } catch {
          await refreshThreads();
        }
        if (removed) {
          append({ type: "status", message: `已删除会话 ${id}` });
        } else {
          append({ type: "status", message: `会话已不存在：${id}` });
        }
      } catch (e) {
        const msg = String(e);
        append({ type: "error", message: `删除失败：${msg}` });
        // 侧栏可见的硬反馈（不用 window.alert）
        setStatusMsg(`删除失败：${msg}`);
      }
    },
    [activeThread, append, refreshThreads, threads],
  );

  const onRenameSession = useCallback(
    async (id: string, title: string) => {
      try {
        await renameThread(id, title);
        await refreshThreads();
      } catch (e) {
        append({ type: "error", message: String(e) });
      }
    },
    [append, refreshThreads],
  );

  const atMatches = useMemo(() => {
    if (!atQuery) return [];
    const q = atQuery.q.toLowerCase();
    return fileEntries
      .filter((p) => p.toLowerCase().includes(q))
      .slice(0, 8);
  }, [atQuery, fileEntries]);

  const applyAtPick = useCallback(
    (path: string) => {
      if (!atQuery) return;
      const ref = formatFileRef(path);
      const before = atQuery.prefix;
      const after = input.slice(before.length);
      // 替换 @… 片段为完整 ref（去掉残缺查询）
      const replaced = `${before}${ref}${after.replace(/^@[\w./-]*/, "")}`;
      setInput(replaced);
      setAtQuery(null);
      setAtIdx(0);
      requestAnimationFrame(() => inputRef.current?.focus());
    },
    [atQuery, input],
  );

  const onInputChange = (value: string) => {
    setInput(value);
    // 斜杠：整行 `/query`（IA-10）
    const sm = /^\/([\w-]*)$/.exec(value.trim());
    if (sm) {
      setAtQuery(null);
      setSlashIdx(0);
      return;
    }
    // 检测光标前的 @ 查询（简化：文末）
    const m = /(?:^|\s)@([\w./-]*)$/.exec(value);
    if (m && fileEntries.length) {
      const idx = value.lastIndexOf("@" + m[1]);
      setAtQuery({ prefix: value.slice(0, idx), q: m[1] });
      setAtIdx(0);
    } else {
      setAtQuery(null);
    }
  };

  /** 整行斜杠候选（IA-10 菜单） */
  const slashOpen = /^\/([\w-]*)$/.test(input.trim());
  const slashQ = slashOpen ? input.trim().slice(1) : "";
  const slashMatches = slashOpen ? matchSlash(slashQ) : [];

  const commands: CommandItem[] = useMemo(
    () => [
      { id: "new", label: "新建会话", hint: "⌘N", run: onNewThread },
      { id: "fork", label: "分叉当前会话", hint: "session/fork", run: onFork },
      { id: "stop", label: "中断当前生成", hint: "Esc", run: stop },
      {
        id: "panel",
        label: "切换右侧面板",
        hint: "⌘J",
        run: () => setPanelOpen((v) => !v),
      },
      {
        id: "sidebar",
        label: "切换侧栏",
        hint: "⌘B",
        run: () => setSidebarOpen((v) => !v),
      },
      {
        id: "focus",
        label: "聚焦输入框",
        hint: "⌘L",
        run: () => inputRef.current?.focus(),
      },
      {
        id: "cycle-mode",
        label: "循环执行模式",
        hint: "⇧Tab",
        run: cycleMode,
      },
      {
        id: "settings",
        label: "打开设置",
        hint: "⌘,",
        run: () => setSettingsOpen(true),
      },
      {
        id: "compact",
        label: "压缩上下文 /compact",
        run: async () => {
          try {
            await compactSession();
            append({ type: "status", message: "已请求上下文压缩" });
          } catch (e) {
            append({ type: "error", message: String(e) });
          }
        },
      },
      {
        id: "export",
        label: "导出对话 markdown",
        hint: "thread/export",
        run: async () => {
          try {
            const r = await threadExport("markdown");
            append({
              type: "status",
              message: `已导出（${(r.content ?? "").length} 字）`,
            });
            if (r.content) {
              append({
                type: "assistant",
                text: "```\n" + r.content.slice(0, 500) + "\n```",
              });
            }
          } catch (e) {
            append({ type: "error", message: String(e) });
          }
        },
      },
      {
        id: "review",
        label: "审查未提交改动",
        hint: "review/start",
        run: async () => {
          try {
            const r = await reviewStart(activeThread || "appserver", {
              type: "uncommittedChanges",
            });
            if (r.prompt) await startTurn(r.prompt);
          } catch (e) {
            append({ type: "error", message: String(e) });
          }
        },
      },
      {
        id: "shell",
        label: "命令台…（输入 shell）",
        hint: "command/exec",
        run: async () => {
          const cmd = window.prompt("shell 命令（不经模型，走沙箱）");
          if (cmd?.trim()) {
            try {
              await commandExec(cmd.trim());
              append({ type: "status", message: `已执行：${cmd.trim()}（结果见工具事件）` });
            } catch (e) {
              append({ type: "error", message: String(e) });
            }
          }
        },
      },
      {
        id: "goal-clear",
        label: "清除目标",
        run: async () => {
          try {
            await goalClear();
          } catch (e) {
            append({ type: "error", message: String(e) });
          }
        },
      },
      ...EXEC_MODES.map((m) => ({
        id: `mode-${m.id}`,
        label: `执行模式 → ${m.label}`,
        run: () => onModeChange(m.id),
      })),
      ...modelsList.map((m) => ({
        id: `model-${m.name}`,
        label: `切换模型 → ${m.name}`,
        run: () => onModelChange(m.name),
      })),
    ],
    [activeThread, append, cycleMode, modelsList, onFork, onModeChange, onModelChange, onNewThread, stop],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key === ",") {
        e.preventDefault();
        setSettingsOpen((v) => !v);
        return;
      }
      if (settingsOpen && e.key === "Escape") {
        e.preventDefault();
        setSettingsOpen(false);
        return;
      }
      if (mod && !e.shiftKey && (e.key === "k" || e.key === "p")) {
        e.preventDefault();
        setPaletteOpen(true);
      } else if (mod && e.key === "b") {
        e.preventDefault();
        setSidebarOpen((v) => !v);
      } else if (mod && e.key === "j") {
        e.preventDefault();
        setPanelOpen((v) => !v);
      } else if (mod && e.key === "n") {
        e.preventDefault();
        void onNewThread();
      } else if (mod && e.key === "l") {
        e.preventDefault();
        inputRef.current?.focus();
      } else if (e.shiftKey && e.key === "Tab") {
        e.preventDefault();
        cycleMode();
      } else if (e.key === "Escape" && !paletteOpen) {
        if (busy || approval) {
          e.preventDefault();
          if (busy) void stop();
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [approval, busy, cycleMode, onNewThread, paletteOpen, settingsOpen, stop]);

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    localStorage.setItem("neo-theme", theme);
  }, [theme]);

  const totalAdd = files.reduce((s, f) => s + f.additions, 0);
  const totalDel = files.reduce((s, f) => s + f.deletions, 0);

  const lastUserText = useMemo(() => {
    for (let i = items.length - 1; i >= 0; i--) {
      const it = items[i];
      if (it.type === "user") return it.text;
    }
    return null;
  }, [items]);

  const onEditLastUser = useCallback(async () => {
    if (lastUserText == null || busy || approval) return;
    try {
      await rewindTurns(1);
      // rewound 事件会截断本地 items；再把原文放回输入框
      setInput(lastUserText);
      requestAnimationFrame(() => inputRef.current?.focus());
    } catch (e) {
      append({ type: "error", message: String(e) });
    }
  }, [append, approval, busy, lastUserText]);

  const hasProject = projectMode === "workspace" && Boolean(workspaceRoot);
  /** 侧栏标题：none → 不在项目中；有项目 → 目录名 */
  const projectLabel = useMemo(() => {
    if (projectMode === "none") return "不在项目中工作";
    if (!workspaceRoot) return "选择项目";
    const parts = workspaceRoot.replace(/\/+$/, "").split("/");
    return parts[parts.length - 1] || workspaceRoot;
  }, [projectMode, workspaceRoot]);
  /** Composer 芯片（ZCode）：有项目显示目录名，否则永远是「选择项目」 */
  const chipLabel = hasProject ? projectLabel : "选择项目";

  const methods = useMemo(() => init?.methods ?? [], [init]);
  const grouped = useMemo(() => groupTools(items), [items]);
  /** IA-21：新建任务态 = 无消息且无审批（与对话底栏分观感） */
  const isNewTask = items.length === 0 && !approval;
  const samplePrompts = [
    "用一句话介绍这个仓库",
    "总结当前分支相对 main 的改动",
    "修一个明显的 bug 并写测试",
  ];

  const onSample = (text: string) => {
    setInput(text);
    inputRef.current?.focus();
  };

  /** IA-11：拖分栏；pointer capture 在 window，松手写入 localStorage */
  const beginSplitDrag = useCallback(
    (edge: SplitEdge, clientX: number) => {
      const startX = clientX;
      const startW = edge === "sidebar" ? sidebarW : panelW;
      const min = edge === "sidebar" ? SIDEBAR_MIN : PANEL_MIN;
      const max = edge === "sidebar" ? SIDEBAR_MAX : PANEL_MAX;
      const apply = (w: number) => {
        const next = clamp(Math.round(w), min, max);
        if (edge === "sidebar") setSidebarW(next);
        else setPanelW(next);
      };
      const onMove = (e: PointerEvent) => {
        const dx = e.clientX - startX;
        // 侧栏拖右变宽；右栏拖左变宽
        apply(edge === "sidebar" ? startW + dx : startW - dx);
      };
      const onUp = (e: PointerEvent) => {
        const dx = e.clientX - startX;
        const w = clamp(Math.round(edge === "sidebar" ? startW + dx : startW - dx), min, max);
        if (edge === "sidebar") {
          setSidebarW(w);
          localStorage.setItem("neo-sidebar-w", String(w));
        } else {
          setPanelW(w);
          localStorage.setItem("neo-panel-w", String(w));
        }
        document.body.classList.remove("is-resizing");
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        window.removeEventListener("pointercancel", onUp);
      };
      document.body.classList.add("is-resizing");
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
      window.addEventListener("pointercancel", onUp);
    },
    [panelW, sidebarW],
  );

  return (
    <>
      {/*
        Overlay 标题条：只挂 data-tauri-drag-region。
        不要再绑 onDoubleClick → toggleMaximize —— 会与 Tauri 原生
        「双击 drag-region 最大化」叠成两次 toggle（放大又缩回，真机踩过）。
      */}
      <div className="titlebar-drag" data-tauri-drag-region aria-hidden />
    <div
      className={`app${panelOpen ? " with-panel" : ""}${sidebarOpen ? "" : " no-sidebar"}`}
      style={
        {
          "--sidebar-w": `${sidebarW}px`,
          "--panel-w": `${panelW}px`,
        } as React.CSSProperties
      }
    >
      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        commands={commands}
      />
      <SettingsModal
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        theme={theme}
        onTheme={setTheme}
        model={model}
        models={modelsList}
        onModel={(n) => void onModelChange(n)}
        execMode={execMode}
        onMode={(m) => void onModeChange(m)}
        protocolVersion={init?.protocol_version ?? "—"}
        methodCount={methods.length}
        workspaceRoot={workspaceRoot}
      />
      {diffModal && (
        <DiffModal
          path={diffModal.path}
          diff={diffModal.diff}
          onClose={() => setDiffModal(null)}
        />
      )}

      <Sidebar
        open={sidebarOpen}
        onToggle={() => setSidebarOpen((v) => !v)}
        onNew={() => void onNewThread()}
        onCommand={() => setPaletteOpen(true)}
        onTheme={() => setTheme((v) => (v === "light" ? "dark" : "light"))}
        onSettings={() => setSettingsOpen(true)}
        theme={theme}
        threads={threads}
        activeId={activeThread}
        onResume={(id) => void onResume(id)}
        onRename={(id, title) => void onRenameSession(id, title)}
        onDelete={(id) => void onDeleteSession(id)}
        onArchive={(id, archived) => {
          void archiveThread(id, archived)
            .then(() => refreshThreads())
            .catch((e) => append({ type: "error", message: String(e) }));
        }}
        sections={sections}
        onCreateSection={(name) => {
          void createSection(name)
            .then(() => refreshSections())
            .catch((e) => append({ type: "error", message: String(e) }));
        }}
        projectLabel={projectLabel}
        workspaceRoot={projectMode === "none" ? "" : workspaceRoot}
        projectMode={projectMode}
        recentWorkspaces={recentWs}
        onSwitchWorkspace={(p) => void switchWorkspace(p)}
        onNoProject={() => void switchNoProject()}
        filesFoot={files.length > 0 ? `改动 +${totalAdd} −${totalDel} · ${files.length} 文件` : undefined}
      />
      {sidebarOpen && (
        <div
          className="v-split"
          role="separator"
          aria-orientation="vertical"
          aria-label="调整侧栏宽度"
          title="拖动调整侧栏宽度"
          onPointerDown={(e) => {
            e.preventDefault();
            beginSplitDrag("sidebar", e.clientX);
          }}
        />
      )}

      <div className="center">
      <main className="stream" ref={streamRef}>
        {goal && !floatCollapsed && (
          <ProgressFloat
            goal={goal}
            busy={busy}
            onCollapse={() => setFloatCollapsed(true)}
          />
        )}
        {goal && floatCollapsed && (
          <button
            type="button"
            className="progress-reopen"
            onClick={() => setFloatCollapsed(false)}
            title="显示进度"
          >
            进程 {goal.subtasks.filter((s) => s.phase === "done").length}/
            {goal.subtasks.length || "?"}
          </button>
        )}
        <div className="stream-inner">
          {isNewTask && (
            <div className="hero">
              <div className="hero-mark" aria-hidden>
                NEO
              </div>
              <h1>接下来交给我吧</h1>
              <p className="hero-sub">
                对接自家 app-server · 本地优先 · 不外传代码
              </p>
              <div className="hero-keys">
                <kbd>⌘K</kbd> 命令 · <kbd>⌘N</kbd> 新任务 · <kbd>/</kbd> 斜杠 ·{" "}
                <kbd>Esc</kbd> 中断 · <kbd>⇧Tab</kbd> 模式
              </div>
              <p className="hero-status">
                {status === "boot"
                  ? "正在连接…"
                  : status === "error"
                    ? statusMsg
                    : `就绪 · ${model} · 协议 v${init?.protocol_version ?? "—"}`}
              </p>
            </div>
          )}

          {items.some((i) => i.type === "reasoning") && (
            <div className="stream-tools">
              <input
                value={thinkingSearch}
                placeholder="搜索思考轨迹…"
                onChange={(e) => setThinkingSearch(e.target.value)}
              />
            </div>
          )}

          {busy && turnStartedAt && (
            <div className="working-line" aria-live="polite">
              工作中 {formatWorkDur(Date.now() - turnStartedAt)}
            </div>
          )}

          {grouped.map((g, i) => {
            if (g.kind === "tools") return <ToolGroup key={`t${i}`} tools={g.tools} />;
            const it = g.item;
            switch (it.type) {
              case "user":
                return (
                  <div key={i} className="msg user">
                    <div className="bubble user">{it.text}</div>
                    {it.text === lastUserText && !busy && !approval && (
                      <button
                        type="button"
                        className="ghost-btn edit-btn"
                        onClick={() => void onEditLastUser()}
                        title="编辑并重发"
                      >
                        编辑
                      </button>
                    )}
                  </div>
                );
              case "assistant":
                return (
                  <div key={i} className="msg assistant">
                    <div className="msg-role">
                      <span>NEO</span>
                      <button
                        type="button"
                        className="ghost-btn edit-btn"
                        onClick={() => {
                          void navigator.clipboard.writeText(it.text);
                          append({ type: "status", message: "已复制回复" });
                        }}
                        title="复制"
                      >
                        复制
                      </button>
                    </div>
                    <div className="bubble assistant">{renderMarkdown(it.text)}</div>
                  </div>
                );
              case "reasoning":
                return (
                  <ThinkingBlock
                    key={i}
                    text={it.text}
                    search={thinkingSearch}
                    forceOpen={Boolean(
                      thinkingSearch &&
                        it.text.toLowerCase().includes(thinkingSearch.toLowerCase()),
                    )}
                  />
                );
              case "error":
                return (
                  <div key={i} className="bubble error">
                    {it.message}
                  </div>
                );
              case "patch": {
                const st = diffStats(it.diff);
                return (
                  <button
                    key={i}
                    type="button"
                    className="edit-line"
                    onClick={() => setDiffModal({ path: it.path, diff: it.diff })}
                    title="打开 Diff 全屏"
                  >
                    <span className="ed-verb">编辑</span>
                    <span className="ed-path">{it.path}</span>
                    <span className="ed-stat">
                      <span className="add">+{st.a}</span>{" "}
                      <span className="del">−{st.d}</span>
                    </span>
                  </button>
                );
              }
              case "turn":
                return null;
              case "summary":
                return (
                  <div key={i} className="turn-meta" title="轮摘要">
                    已处理 <b>{(it.ms / 1000).toFixed(0)}s</b>
                    <span className="dim"> · +{it.input_tokens}/−{it.output_tokens} tok</span>
                  </div>
                );
              case "files": {
                if (!it.files.length) return null;
                const fa = it.files.reduce((s, f) => s + f.additions, 0);
                const fd = it.files.reduce((s, f) => s + f.deletions, 0);
                const first = it.files[0];
                return (
                  <div key={i} className="edit-line static" title={it.files.map((f) => f.path).join("\n")}>
                    <span className="ed-verb">编辑</span>
                    <span className="ed-path">
                      {first.path}
                      {it.files.length > 1 ? ` 等 ${it.files.length} 文件` : ""}
                    </span>
                    <span className="ed-stat">
                      <span className="add">+{fa}</span>{" "}
                      <span className="del">−{fd}</span>
                    </span>
                  </div>
                );
              }
              case "status":
                return (
                  <div key={i} className="bubble reasoning">
                    {it.message}
                  </div>
                );
              default:
                return null;
            }
          })}

          {approval && (
            <div className="approval" role="alertdialog">
              <div className="title">需要审批 · {approval.kind}</div>
              <div className="detail">{approval.detail}</div>
              <div className="actions">
                <button
                  type="button"
                  className="primary"
                  onClick={() => void onApproval("allow")}
                >
                  允许
                </button>
                <button
                  type="button"
                  onClick={() => void onApproval("allow", true)}
                  title="approval/respondStep — 只执行本步剩余，不驱动后续"
                >
                  仅本步
                </button>
                <button
                  type="button"
                  onClick={() => void onApproval("allow_always")}
                >
                  总是允许
                </button>
                <button
                  type="button"
                  className="danger"
                  onClick={() => void onApproval("deny")}
                >
                  拒绝
                </button>
              </div>
              <p className="muted">输入区已锁定（ZCode：审批时阻塞 composer）</p>
            </div>
          )}

          {userInput && (
            <div className="approval user-input" role="alertdialog" aria-label="需要你的输入">
              <div className="title">需要你的输入</div>
              <div className="detail">{userInput.prompt}</div>
              <div className="user-input-row">
                <input
                  value={userInputText}
                  placeholder="输入答复…"
                  autoFocus
                  onChange={(e) => setUserInputText(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      void onUserInputSubmit();
                    }
                  }}
                />
                <button
                  type="button"
                  className="primary"
                  disabled={!userInputText.trim()}
                  onClick={() => void onUserInputSubmit()}
                >
                  发送答复
                </button>
              </div>
              <p className="muted">user_input/respond · 输入区已锁定</p>
            </div>
          )}

          {stderrLines.length > 0 && (
            <details className="bubble reasoning">
              <summary>stderr（{stderrLines.length}）</summary>
              {stderrLines.join("\n")}
            </details>
          )}
        </div>
      </main>

      <footer className={`composer ${isNewTask ? "mode-new" : "mode-chat"}`}>
        {/* IA-36：仅欢迎页显示项目选择器；进入对话后输入框上方清空 */}
        {isNewTask && (
        <div className="ctx-chips" aria-label="工作区上下文">
          <div className="ctx-anchor">
            <button
              type="button"
              className="ctx-chip"
              aria-haspopup="dialog"
              aria-expanded={projPickerOpen}
              title={hasProject ? workspaceRoot : "选择工作区"}
              onMouseDown={(e) => {
                // 防止 document mousedown 先关菜单
                e.stopPropagation();
              }}
              onClick={(e) => {
                const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
                setProjAnchor({ left: r.left, top: r.top });
                setProjPickerOpen((v) => !v);
              }}
            >
              {hasProject ? (
                <span
                  className="chip-x"
                  role="button"
                  tabIndex={0}
                  aria-label="改为不在项目中工作"
                  onClick={(e) => {
                    e.stopPropagation();
                    void switchNoProject();
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.stopPropagation();
                      void switchNoProject();
                    }
                  }}
                >
                  <Icon name="close" size={12} />
                </span>
              ) : (
                <Icon name="files" size={12} />
              )}
              {chipLabel}
              <Icon name="chevron-down" size={11} />
            </button>
            <ProjectPicker
              open={projPickerOpen}
              onClose={() => setProjPickerOpen(false)}
              projectMode={projectMode}
              projectLabel={projectLabel}
              workspaceRoot={workspaceRoot}
              recents={recentWs}
              onSwitch={(p) => void switchWorkspace(p)}
              onNoProject={() => void switchNoProject()}
              anchor={projAnchor}
            />
          </div>
          {hasProject && branch && branch !== "—" && branch.trim() !== "" && (
            <span className="ctx-chip muted" title={`当前分支 ${branch}`}>
              <Icon name="goal" size={12} />
              {branch}
            </span>
          )}
        </div>
        )}
        <div className="composer-card">
          {webPicks.length > 0 && (
            <div className="attach-chips" role="list" aria-label="网页元素附件">
              <span
                className="attach-chip"
                role="listitem"
                title={webPicks.map((e) => `${e.tag} ${e.selector}`).join("\n")}
              >
                <Icon name="browser" size={13} />
                <span className="attach-label">
                  {webPicks.length === 1
                    ? `${webPicks[0].tag} 元素`
                    : `${webPicks.length} 个网页元素`}
                </span>
                <button
                  type="button"
                  className="attach-x"
                  aria-label="移除全部网页元素"
                  onClick={() => setWebPicks([])}
                >
                  <Icon name="close" size={12} />
                </button>
              </span>
              {webPicks.length > 1 && (
                <span className="attach-more">+{webPicks.length - 1}</span>
              )}
            </div>
          )}
          <div className="at-wrap">
            <textarea
              ref={inputRef}
              value={input}
              placeholder={
                approval
                  ? "待审批 — 输入已锁定"
                  : userInput
                    ? "待答复 — 输入已锁定"
                    : webPicks.length
                      ? "对选中的网页元素提出后续修改要求…"
                      : isNewTask
                        ? "描述要做的任务，用 @ 引用文件、/ 命令，或 ⌘K…"
                        : busy
                          ? "生成中 — Enter 转向当前轮（steer）"
                          : "继续输入…"
              }
              onChange={(e) => onInputChange(e.target.value)}
              onKeyDown={(e) => {
                if (slashOpen && slashMatches.length) {
                  if (e.key === "ArrowDown") {
                    e.preventDefault();
                    setSlashIdx((i) => Math.min(i + 1, slashMatches.length - 1));
                    return;
                  }
                  if (e.key === "ArrowUp") {
                    e.preventDefault();
                    setSlashIdx((i) => Math.max(i - 1, 0));
                    return;
                  }
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    const pick = slashMatches[Math.min(slashIdx, slashMatches.length - 1)];
                    if (pick) {
                      setInput(`/${pick.name}`);
                      // 下一拍由 send 执行（或用户继续输入）
                      void slashRunnerRef.current(`/${pick.name}`);
                      setInput("");
                    }
                    return;
                  }
                  if (e.key === "Escape") {
                    e.preventDefault();
                    setInput("");
                    return;
                  }
                }
                if (atQuery && atMatches.length) {
                  if (e.key === "ArrowDown") {
                    e.preventDefault();
                    setAtIdx((i) => Math.min(i + 1, atMatches.length - 1));
                    return;
                  }
                  if (e.key === "ArrowUp") {
                    e.preventDefault();
                    setAtIdx((i) => Math.max(i - 1, 0));
                    return;
                  }
                  if (e.key === "Enter" || e.key === "Tab") {
                    e.preventDefault();
                    applyAtPick(atMatches[atIdx]);
                    return;
                  }
                  if (e.key === "Escape") {
                    e.preventDefault();
                    setAtQuery(null);
                    return;
                  }
                }
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  void send();
                }
                if (e.key === "Escape" && !approval && !userInput) {
                  e.preventDefault();
                  if (busy) void stop();
                }
              }}
              disabled={blocked || (status !== "ready" && status !== "busy")}
              rows={3}
            />
            {atQuery && atMatches.length > 0 && (
              <ul className="at-menu" role="listbox">
                {atMatches.map((p, i) => (
                  <li key={p}>
                    <button
                      type="button"
                      className={i === atIdx ? "active" : ""}
                      onMouseEnter={() => setAtIdx(i)}
                      onClick={() => applyAtPick(p)}
                    >
                      {p}
                    </button>
                  </li>
                ))}
              </ul>
            )}
            {slashOpen && slashMatches.length > 0 && !atQuery && (
              <ul className="at-menu slash-menu" role="listbox" aria-label="斜杠命令">
                {slashMatches.map((c, i) => (
                  <li key={c.id}>
                    <button
                      type="button"
                      className={i === Math.min(slashIdx, slashMatches.length - 1) ? "active" : ""}
                      onMouseEnter={() => setSlashIdx(i)}
                      onClick={() => {
                        void slashRunnerRef.current(`/${c.name}`);
                        setInput("");
                      }}
                    >
                      <span className="cmd">/{c.name}</span>
                      <span className="desc">{c.desc}</span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
          <div className="composer-bar">
            <button
              type="button"
              className="mode-chip"
              disabled={busy}
              onClick={cycleMode}
              title="⇧Tab 切换模式"
            >
              <span className="mode-dot" aria-hidden />
              {EXEC_MODES.find((m) => m.id === execMode)?.label ?? execMode}
            </button>
            <span className="hint">
              {approval
                ? "等待审批"
                : userInput
                  ? "等待答复"
                  : busy
                    ? "生成中 · Enter 转向 · Esc 中断"
                    : "Enter 发送"}
              {lastSummary ? ` · 上轮 +${lastSummary.in}/−${lastSummary.out}` : ""}
            </span>
            <Select
              className="model-inline"
              value={model}
              disabled={busy}
              title="模型"
              ariaLabel="模型"
              options={(modelsList.length ? modelsList : [{ name: model }]).map((m) => ({
                value: m.name,
                label: m.name,
              }))}
              onChange={(v) => void onModelChange(v)}
            />
            <button
              type="button"
              className="send-circle"
              onClick={() => {
                // busy 且无输入 = 中断；有输入 = 转向（steer）
                if (busy && !input.trim() && webPicks.length === 0) void stop();
                else void send();
              }}
              disabled={
                blocked ||
                (status !== "ready" && status !== "busy") ||
                (!busy && !input.trim() && webPicks.length === 0)
              }
              aria-label={busy && !input.trim() ? "中断" : "发送"}
              title={
                busy
                  ? input.trim()
                    ? "转向当前轮 turn/steer"
                    : "中断 Esc"
                  : "发送 Enter"
              }
            >
              <Icon
                name={busy && !input.trim() && webPicks.length === 0 ? "stop" : "send"}
                size={16}
              />
            </button>
            {busy && (
              <button type="button" className="ghost-btn" onClick={() => void stop()}>
                停止
              </button>
            )}
          </div>
        </div>
        {/* 仅新建任务态：建议条在输入**下方**（ZCode 同构；对话态不重复） */}
        {isNewTask && (
          <div className="suggest-row" role="list">
            {samplePrompts.map((s) => (
              <button
                key={s}
                type="button"
                className="suggest-chip"
                role="listitem"
                onClick={() => onSample(s)}
              >
                {s}
              </button>
            ))}
          </div>
        )}
      </footer>
      </div>
      {/* 右侧：浏览器式标签页（× 关闭 · + 下拉开新），不是全部 tab 挤一行 */}
      {panelOpen && openTabs.length > 0 && (
        <>
          <div
            className="v-split"
            role="separator"
            aria-orientation="vertical"
            aria-label="调整右栏宽度"
            title="拖动调整右栏宽度"
            onPointerDown={(e) => {
              e.preventDefault();
              beginSplitDrag("panel", e.clientX);
            }}
          />
          <WorkbenchShell
            openTabs={openTabs}
            active={panelTab}
            onActive={setPanelTab}
            onCloseTab={(id) => closeWorkbenchTab(id)}
            onOpen={(id) => openWorkbench(id)}
            onClosePanel={() => setPanelOpen(false)}
          >
            {panelTab === "review" && (
              <div className="wb-section">
                <div className="wb-sub">
                  改动文件
                  <button
                    type="button"
                    className="ghost-btn"
                    style={{ marginLeft: "auto" }}
                    disabled={busy}
                    onClick={() => {
                      void (async () => {
                        try {
                          const r = await reviewStart(
                            activeThread || "appserver",
                            { type: "uncommittedChanges" },
                          );
                          if (r.prompt) {
                            append({
                              type: "status",
                              message: "review/start → 发送审查任务",
                            });
                            await startTurn(r.prompt);
                          }
                        } catch (e) {
                          append({ type: "error", message: String(e) });
                        }
                      })();
                    }}
                  >
                    开始审查
                  </button>
                </div>
                {files.length === 0 ? (
                  <p className="muted pad">本轮尚无文件变更</p>
                ) : (
                  <ul className="file-list flush">
                    {files.map((f) => (
                      <li key={f.path}>
                        <span className="path" title={f.path}>{f.path}</span>
                        <span className="stat">
                          <span className="add">+{f.additions}</span>{" "}
                          <span className="del">−{f.deletions}</span>
                        </span>
                      </li>
                    ))}
                  </ul>
                )}
                <div className="wb-sub">Diff</div>
                {lastPatch ? (
                  <>
                    <div className="panel-path">
                      {lastPatch.path}{" "}
                      <button type="button" className="ghost-btn" onClick={() => setDiffModal({ ...lastPatch })}>
                        全屏
                      </button>
                    </div>
                    <DiffView diff={lastPatch.diff} />
                  </>
                ) : (
                  <p className="muted pad">审批预览 / apply_patch 会出现在这里</p>
                )}
              </div>
            )}

            {panelTab === "terminal" && (
              <div className="wb-section terminal">
                <div className="term-note">
                  命令台 · 每条一个进程 · 走沙箱 · 不经模型（非交互式 PTY）
                </div>
                <div className="term-log">
                  {termLog.length === 0 && (
                    <div className="muted">输入命令回车执行，stdout/stderr 显示在下方。</div>
                  )}
                  {termLog.map((row, i) => (
                    <div
                      key={row.id ?? i}
                      className={`term-line ${row.ok === false ? "fail" : row.ok ? "ok" : "running"}`}
                    >
                      <div>
                        <span className="prompt">$</span> {row.cmd}
                        {row.ok === null && <span className="dim"> ·…</span>}
                        {row.ok === false && <span className="err"> ✗</span>}
                        {row.ok === true && <span className="ok-mark"> ✓</span>}
                      </div>
                      {row.stdout && <pre className="term-out">{row.stdout}</pre>}
                      {row.stderr && <pre className="term-err">{row.stderr}</pre>}
                      {row.truncated && (
                        <div className="muted term-trunc">输出已截断（有界）</div>
                      )}
                    </div>
                  ))}
                </div>
                <form
                  className="term-input"
                  onSubmit={(e) => {
                    e.preventDefault();
                    void runTerminal(termInput);
                  }}
                >
                  <span className="prompt">$</span>
                  <input
                    value={termInput}
                    onChange={(e) => setTermInput(e.target.value)}
                    placeholder="ls -la"
                    spellCheck={false}
                    autoComplete="off"
                  />
                </form>
              </div>
            )}

            {panelTab === "subagents" && (
              <div className="wb-section">
                <div className="wb-sub">子智能体 · tools/list 中的 agent_*</div>
                <p className="muted pad">
                  定义放 <code>.neo/agents/*.md</code>（或 <code>~/.neo/agents/</code>）；
                  装配期加载，新增需重启 app-server。主对话里模型可直接调用这些工具。
                </p>
                {agentTools.length === 0 ? (
                  <p className="muted pad">当前工作区未注册子代理。</p>
                ) : (
                  <ul className="agent-list">
                    {agentTools.map((t) => (
                      <li key={t.name}>
                        <div className="agent-name">{t.name}</div>
                        <div className="agent-desc">{t.description || "—"}</div>
                        <button
                          type="button"
                          className="ghost-btn"
                          onClick={() => {
                            setInput(
                              (v) =>
                                (v ? `${v.trimEnd()}\n` : "") +
                                `请调用 ${t.name} 处理：`,
                            );
                            inputRef.current?.focus();
                          }}
                        >
                          插入任务
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
              </div>
            )}

            {panelTab === "automations" && (
              <div className="wb-section">
                <div className="wb-sub">Automations · 排程提示</div>
                <p className="muted pad">
                  仅本机 · 桌面存活时触发 · 到点在空闲时 <code>turn/start</code>（非 OS cron）。
                </p>
                <div className="auto-form">
                  <input
                    value={autoTitle}
                    placeholder="标题，如 每日站会摘要"
                    onChange={(e) => setAutoTitle(e.target.value)}
                  />
                  <textarea
                    value={autoPrompt}
                    placeholder="交给模型的提示词…"
                    rows={3}
                    onChange={(e) => setAutoPrompt(e.target.value)}
                  />
                  <div className="auto-row">
                    <label>
                      每隔
                      <Select
                        value={String(autoEvery)}
                        ariaLabel="间隔"
                        options={[
                          { value: "0", label: "不自动（仅手动）" },
                          { value: "5", label: "5 分钟" },
                          { value: "15", label: "15 分钟" },
                          { value: "30", label: "30 分钟" },
                          { value: "60", label: "60 分钟" },
                          { value: "180", label: "3 小时" },
                          { value: "720", label: "12 小时" },
                        ]}
                        onChange={(v) => setAutoEvery(Number(v))}
                      />
                    </label>
                    <button
                      type="button"
                      className="primary"
                      disabled={!autoPrompt.trim()}
                      onClick={() => {
                        const title = autoTitle.trim() || "未命名任务";
                        const prompt = autoPrompt.trim();
                        if (!prompt) return;
                        const a: Automation = {
                          id: `a-${Date.now().toString(36)}`,
                          title,
                          prompt,
                          everyMin: autoEvery,
                          enabled: true,
                          nextRunMs:
                            autoEvery > 0 ? Date.now() : 0,
                        };
                        persistAutos([a, ...autos]);
                        setAutoTitle("");
                        setAutoPrompt("");
                      }}
                    >
                      新建
                    </button>
                  </div>
                </div>
                {autos.length === 0 ? (
                  <p className="muted pad">尚无排程任务。</p>
                ) : (
                  <ul className="auto-list">
                    {autos.map((a) => {
                      const nd = a.enabled ? nextDue(a) : 0;
                      const when =
                        a.everyMin <= 0
                          ? "仅手动"
                          : nd
                            ? `下次 ${new Date(nd).toLocaleTimeString()}`
                            : "就绪";
                      return (
                        <li key={a.id}>
                          <div className="auto-title">
                            {a.title}
                            {!a.enabled && <span className="badge">暂停</span>}
                          </div>
                          <div className="auto-prompt">{a.prompt}</div>
                          <div className="auto-meta">
                            {a.everyMin > 0 ? `每 ${a.everyMin} 分 · ` : ""}
                            {when}
                            {a.lastRunMs
                              ? ` · 上次 ${new Date(a.lastRunMs).toLocaleString()}`
                              : ""}
                          </div>
                          {a.lastError && (
                            <div className="auto-err">{a.lastError}</div>
                          )}
                          <div className="auto-actions">
                            <button
                              type="button"
                              className="ghost-btn"
                              disabled={busy}
                              onClick={() => void runAutomation(a.id)}
                            >
                              立即运行
                            </button>
                            <button
                              type="button"
                              className="ghost-btn"
                              onClick={() =>
                                persistAutos(
                                  autos.map((x) =>
                                    x.id === a.id
                                      ? {
                                          ...x,
                                          enabled: !x.enabled,
                                          nextRunMs: !x.enabled
                                            ? Date.now() +
                                              (x.everyMin > 0
                                                ? x.everyMin * 60_000
                                                : 0)
                                            : x.nextRunMs,
                                        }
                                      : x,
                                  ),
                                )
                              }
                            >
                              {a.enabled ? "暂停" : "启用"}
                            </button>
                            <button
                              type="button"
                              className="ghost-btn danger"
                              onClick={() => {
                                if (
                                  window.confirm(
                                    `删除排程「${a.title}」？`,
                                  )
                                ) {
                                  persistAutos(
                                    autos.filter((x) => x.id !== a.id),
                                  );
                                }
                              }}
                            >
                              删除
                            </button>
                          </div>
                        </li>
                      );
                    })}
                  </ul>
                )}
              </div>
            )}

            {panelTab === "files" && (
              <div className="wb-section">
                <FileTree
                  root={workspaceRoot || undefined}
                  selected={filePreview?.path ?? null}
                  refreshKey={fileTreeKey}
                  onSelect={(p) => {
                    setPreviewLoading(true);
                    void readWorkspaceFile(workspaceRoot || undefined, p)
                      .then((r) => setFilePreview(r))
                      .catch((e) =>
                        setFilePreview({ path: p, binary: false, content: String(e), bytes: 0 }),
                      )
                      .finally(() => setPreviewLoading(false));
                  }}
                />
                {filePreview && (
                  <div className="file-preview">
                    <div className="preview-head">
                      <span className="path" title={filePreview.path}>{filePreview.path}</span>
                      <button
                        type="button"
                        className="primary"
                        onClick={() => {
                          const ref = formatFileRef(filePreview.path);
                          if (!ref) return;
                          setInput((v) => (v ? `${v.trimEnd()} ${ref}` : ref));
                          inputRef.current?.focus();
                        }}
                      >
                        加入引用
                      </button>
                    </div>
                    {previewLoading ? (
                      <p className="muted">读取中…</p>
                    ) : filePreview.binary ? (
                      <p className="muted">二进制文件</p>
                    ) : (
                      <pre className="preview-body">{filePreview.content ?? ""}</pre>
                    )}
                  </div>
                )}
              </div>
            )}

            {panelTab === "browser" && (
              <BrowserPane
                filePreview={filePreview}
                workspaceRoot={workspaceRoot || undefined}
                onPickElement={(el) => {
                  setWebPicks((prev) => [...prev, el].slice(-6));
                  requestAnimationFrame(() => inputRef.current?.focus());
                  append({
                    type: "status",
                    message: `已附加网页元素 ${el.tag}${el.selector ? ` ${el.selector}` : ""}`,
                  });
                }}
              />
            )}

            {panelTab === "chat" && (
              <div className="wb-section side-chat">
                <div className="wb-sub">侧边聊天 · 当前会话摘要</div>
                <div className="chat-log">
                  {items.filter((i) => i.type === "user" || i.type === "assistant" || i.type === "error").slice(-30).map((it, i) =>
                    it.type === "user" ? (
                      <div key={i} className="chat-row user"><b>你</b> {it.text}</div>
                    ) : it.type === "error" ? (
                      <div key={i} className="chat-row err">{it.message}</div>
                    ) : (
                      <div key={i} className="chat-row bot">
                        <b>NEO</b> {it.text.slice(0, 280)}
                        {it.text.length > 280 ? "…" : ""}
                      </div>
                    ),
                  )}
                  {items.every((i) => i.type === "turn" || i.type === "summary" || i.type === "files") && (
                    <p className="muted pad">暂无对话 — 在中栏输入任务</p>
                  )}
                </div>
                {approval && (
                  <div className="approval compact">
                    <div className="title">待审批 · {approval.kind}</div>
                    <div className="detail">{approval.detail}</div>
                    <div className="actions">
                      <button type="button" className="primary" onClick={() => void onApproval("allow")}>允许</button>
                      <button type="button" className="danger" onClick={() => void onApproval("deny")}>拒绝</button>
                    </div>
                  </div>
                )}
              </div>
            )}

            {panelTab === "sim" && (
              <div className="wb-section sim">
                <div className="wb-sub">模拟器</div>
                <p className="muted pad">设备/场景模拟器 — P2 占位。当前连接：</p>
                <ul className="sim-list">
                  <li>app-server · v{init?.protocol_version ?? "—"}</li>
                  <li>model · {model}</li>
                  <li>mode · {execMode}</li>
                  <li>branch · {branch || "—"}</li>
                </ul>
                <div className="mode-grid">
                  {EXEC_MODES.map((m) => (
                    <button
                      key={m.id}
                      type="button"
                      className={execMode === m.id ? "primary" : ""}
                      onClick={() => void onModeChange(m.id)}
                      disabled={busy}
                    >
                      {m.label}
                    </button>
                  ))}
                </div>
              </div>
            )}

            {panelTab === "goal" && (
              <div className="wb-section">
                <div className="goal-form">
                  <input
                    value={goalInput}
                    placeholder="设定目标，每行一个子任务…"
                    onChange={(e) => setGoalInput(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") void onSetGoal();
                    }}
                  />
                  <button type="button" className="primary" onClick={() => void onSetGoal()}>
                    设定
                  </button>
                </div>
                {goal ? (
                  <div className="goal-card flat">
                    <div className="goal-text">{goal.goal}</div>
                    <div className="muted">
                      {goal.paused ? "已暂停" : goal.stopped ? `已停` : "运行中"}
                      {" · iter "}
                      {goal.iterations} · 剩余 {goal.turns_remaining} 轮
                    </div>
                    <ul className="goal-subtasks">
                      {goal.subtasks.map((s) => (
                        <li key={s.id}>
                          <span className="phase">{s.phase ?? "—"}</span> {s.title}
                        </li>
                      ))}
                    </ul>
                    <div className="mode-grid">
                      <button
                        type="button"
                        onClick={() => void (goal.paused ? goalResume(goal.goal_id) : goalPause(goal.goal_id))}
                      >
                        {goal.paused ? "恢复" : "暂停"}
                      </button>
                      <button type="button" onClick={() => void goalClear()}>清除</button>
                    </div>
                  </div>
                ) : (
                  <p className="muted pad">未设定目标</p>
                )}
                <div className="wb-sub">连接</div>
                <p className="muted mono pad">
                  {init?.server?.name ?? "neo-app-server"} · {methods.length} methods
                </p>
              </div>
            )}
          </WorkbenchShell>
        </>
      )}
      </div>
    </>
  );
}
