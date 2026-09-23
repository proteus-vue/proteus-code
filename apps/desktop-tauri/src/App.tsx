import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  ApprovalState,
  Decision,
  FileChange,
  GoalSnapshot,
  InitializeResult,
  ThreadSummary,
  UiItem,
  WireEvent,
} from "./lib/protocol";
import { renderMarkdown } from "./lib/markdown";
import { formatRelative } from "./lib/time";
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
import { RepoWiki } from "./components/RepoWiki";
import {
  ThinkingBlock,
  ToolGroup,
  groupTools,
} from "./components/TranscriptParts";
import {
  commandExec,
  compactSession,
  configureSession,
  createThread,
  type ExecMode,
  forkThread,
  gitInfo,
  goalClear,
  goalPause,
  goalResume,
  goalSet,
  interruptTurn,
  listModels,
  listThreads,
  onEvent,
  onExit,
  onStderr,
  respondApproval,
  resumeThread,
  rewindTurns,
  startServer,
  startTurn,
  stopServer,
  unpackEvent,
} from "./lib/rpc";
import { listWorkspace } from "./components/FileTree";
import "./styles/tokens.css";
import "./styles/app.css";

type Status = "boot" | "ready" | "busy" | "error";

const EXEC_MODES: { id: ExecMode; label: string }[] = [
  { id: "plan", label: "Plan" },
  { id: "confirm_before", label: "确认" },
  { id: "default", label: "Default" },
  { id: "auto_edit", label: "自动编辑" },
  { id: "full_access", label: "完全访问" },
];

export default function App() {
  const [status, setStatus] = useState<Status>("boot");
  const [statusMsg, setStatusMsg] = useState("正在连接 app-server…");
  const [init, setInit] = useState<InitializeResult | null>(null);
  const [threads, setThreads] = useState<ThreadSummary[]>([]);
  const [activeThread, setActiveThread] = useState<string | null>(null);
  const [model, setModel] = useState<string>("—");
  const [modelsList, setModelsList] = useState<
    { name: string; description?: string; production?: boolean }[]
  >([]);
  const [branch, setBranch] = useState<string>("—");
  const [workspaceRoot, setWorkspaceRoot] = useState<string>("");
  const [items, setItems] = useState<UiItem[]>([]);
  const [approval, setApproval] = useState<ApprovalState | null>(null);
  const [input, setInput] = useState("");
  const [stderrLines, setStderrLines] = useState<string[]>([]);
  const [execMode, setExecMode] = useState<ExecMode>("default");
  const [panelOpen, setPanelOpen] = useState(true);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [lastPatch, setLastPatch] = useState<{ path: string; diff: string } | null>(null);
  const [files, setFiles] = useState<FileChange[]>([]);
  const [goal, setGoal] = useState<GoalSnapshot | null>(null);
  const [goalInput, setGoalInput] = useState("");
  const [thinkingSearch, setThinkingSearch] = useState("");
  const [threadQuery, setThreadQuery] = useState("");
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [panelTab, setPanelTab] = useState<
    "goal" | "files" | "changes" | "diff" | "wiki"
  >("goal");
  const [diffModal, setDiffModal] = useState<{ path: string; diff: string } | null>(null);
  const [filePreview, setFilePreview] = useState<FilePreview | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [floatCollapsed, setFloatCollapsed] = useState(false);
  const [fileEntries, setFileEntries] = useState<string[]>([]);
  const [atQuery, setAtQuery] = useState<{ prefix: string; q: string } | null>(null);
  const [atIdx, setAtIdx] = useState(0);
  const [turnStartedAt, setTurnStartedAt] = useState<number | null>(null);
  const [lastSummary, setLastSummary] = useState<{
    in: number;
    out: number;
    ms: number;
  } | null>(null);
  const streamRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const busy = status === "busy";
  const blocked = busy || !!approval;

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
        case "tool_call_begin":
          append({
            type: "tool",
            id: String(payload.id ?? ""),
            name: String(payload.name ?? "tool"),
            args: payload.arguments,
            status: "running",
          });
          break;
        case "tool_call_end":
          setItems((prev) =>
            prev.map((it) => {
              if (it.type === "tool" && it.id === payload.id) {
                const code = Number(payload.exit_code ?? -1);
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
        case "file_changed": {
          if (kind === "files_changed") {
            const fs = (payload.files as FileChange[] | undefined) ?? [];
            setFiles(fs);
            append({ type: "files", files: fs });
          }
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

  const boot = useCallback(async () => {
    setStatus("boot");
    setStatusMsg("正在连接 app-server…");
    try {
      const info = await startServer({
        provider: "mock",
        workspace: import.meta.env.VITE_NEO_WORKSPACE || undefined,
      });
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
      const g = await gitInfo().catch(() => null);
      if (g && typeof g.branch === "string") setBranch(g.branch);
      if (g && typeof g.root === "string") {
        setWorkspaceRoot(g.root);
        void listWorkspace(g.root)
          .then((r) =>
            setFileEntries(r.entries.filter((e) => !e.endsWith("/"))),
          )
          .catch(() => setFileEntries([]));
      }
      await refreshThreads();
      setStatus("ready");
      setStatusMsg("已连接 · mock provider");
    } catch (e) {
      setStatus("error");
      setStatusMsg(String(e));
    }
  }, [refreshThreads]);

  useEffect(() => {
    const unsubs: Array<() => void> = [];
    void (async () => {
      unsubs.push(
        await onEvent((e) => handleStable(e)),
        await onStderr((line) =>
          setStderrLines((ls) => [...ls.slice(-40), line]),
        ),
        await onExit(() => {
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

  const send = useCallback(async () => {
    const text = input.trim();
    if (!text || blocked) return;
    setInput("");
    try {
      await startTurn(text);
    } catch (e) {
      append({ type: "error", message: String(e) });
      setStatus("error");
    }
  }, [append, blocked, input]);

  const stop = useCallback(async () => {
    try {
      await interruptTurn();
    } catch (e) {
      append({ type: "error", message: String(e) });
    }
  }, [append]);

  const onApproval = useCallback(
    async (decision: Decision) => {
      if (!approval) return;
      const id = approval.id;
      setApproval(null);
      try {
        await respondApproval(id, decision, null);
      } catch (e) {
        append({ type: "error", message: String(e) });
      }
    },
    [approval, append],
  );

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
      setApproval(null);
      setLastPatch(null);
      setGoal(null);
      setFiles([]);
      await refreshThreads();
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
        setLastPatch(null);
        await resumeThread(id);
        await refreshThreads();
        setStatusMsg(`已切换 ${id}`);
      } catch (e) {
        append({ type: "error", message: String(e) });
      }
    },
    [append, refreshThreads],
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
    [append, cycleMode, modelsList, onFork, onModeChange, onModelChange, onNewThread, stop],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
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
  }, [approval, busy, cycleMode, onNewThread, paletteOpen, stop]);

  const [theme, setTheme] = useState<"light" | "dark">(
    () =>
      (localStorage.getItem("neo-theme") as "light" | "dark" | null) ??
      "light",
  );

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

  const projectLabel = useMemo(() => {
    if (!workspaceRoot) return "当前项目";
    const parts = workspaceRoot.replace(/\/+$/, "").split("/");
    return parts[parts.length - 1] || workspaceRoot;
  }, [workspaceRoot]);

  const methods = useMemo(() => init?.methods ?? [], [init]);
  const grouped = useMemo(() => groupTools(items), [items]);
  const samplePrompts = [
    "用一句话介绍这个仓库",
    "总结当前分支相对 main 的改动",
    "修一个明显的 bug 并写测试",
  ];

  const onSample = (text: string) => {
    setInput(text);
    inputRef.current?.focus();
  };

  return (
    <div
      className={`app${panelOpen ? " with-panel" : ""}${sidebarOpen ? "" : " no-sidebar"}`}
    >
      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        commands={commands}
      />
      {diffModal && (
        <DiffModal
          path={diffModal.path}
          diff={diffModal.diff}
          onClose={() => setDiffModal(null)}
        />
      )}

      {sidebarOpen && (
        <aside className="sidebar">
          <div className="brand-row">
            <span className="brand-mark" aria-hidden>
              N
            </span>
            <span className="brand-name">NEO</span>
            <span className="brand-sub">Desktop</span>
          </div>
          <div className="sidebar-actions">
            <button
              type="button"
              className="btn-new"
              onClick={() => void onNewThread()}
              title="⌘N"
            >
              <span aria-hidden>＋</span> 新建任务
            </button>
            <div className="sidebar-tools">
              <button
                type="button"
                className="tool-btn"
                onClick={() => setPaletteOpen(true)}
                title="⌘K 命令"
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><circle cx="11" cy="11" r="7"/><path d="m20 20-3-3"/></svg>
                <span>搜索</span>
              </button>
              <button
                type="button"
                className="tool-btn"
                onClick={() => void onNewThread()}
                title="⌘N"
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M12 5v14M5 12h14"/></svg>
                <span>新建</span>
              </button>
            </div>
          </div>
          <div className="sidebar-section">
            <span className="proj-label" title={workspaceRoot}>
              ▸ {projectLabel}
            </span>
          </div>
          <div className="sidebar-search">
            <input
              value={threadQuery}
              placeholder="搜索会话…"
              onChange={(e) => setThreadQuery(e.target.value)}
            />
          </div>
          <ul className="sidebar-list">
            {threads.length === 0 && (
              <li>
                <button type="button" className="active">
                  <span className="row1">
                    <span className="dot st-idle" />
                    当前任务
                  </span>
                  <span className="meta">尚未落盘</span>
                </button>
              </li>
            )}
            {threads
              .filter((t) => {
                const s = threadQuery.trim().toLowerCase();
                if (!s) return true;
                return (
                  (t.title ?? "").toLowerCase().includes(s) ||
                  t.id.toLowerCase().includes(s)
                );
              })
              .map((t) => {
                const rel = formatRelative(t.updated_ms);
                return (
              <li key={t.id}>
                <button
                  type="button"
                  className={activeThread === t.id ? "active" : ""}
                  onClick={() => void onResume(t.id)}
                >
                  <span className="row1">
                    <span className={`dot st-${t.state ?? "empty"}`} />
                    <span
                      className={`title ${t.has_title === false ? "faded" : ""}`}
                    >
                      {t.title || t.id}
                    </span>
                    {t.additions != null &&
                      t.deletions != null &&
                      t.additions + t.deletions > 0 && (
                        <span className="delta">
                          +{t.additions} −{t.deletions}
                        </span>
                      )}
                    {rel && <span className="rel">{rel}</span>}
                  </span>
                  <span className="meta">
                    {t.state ?? "empty"}
                    {t.records != null && t.records > 0 && ` · ${t.records} 条`}
                  </span>
                </button>
              </li>
                );
              })}
          </ul>
          {files.length > 0 && (
            <div className="sidebar-foot">
              改动 +{totalAdd} −{totalDel} · {files.length} 文件
            </div>
          )}
        </aside>
      )}

      <header className="toolbar">
        {!sidebarOpen && (
          <button type="button" className="icon-btn" onClick={() => setSidebarOpen(true)} title="⌘B">
            ☰
          </button>
        )}
        <div className="tb-left">
          <span className="tb-workspace" title={workspaceRoot || branch}>
            <span className="tb-icon">⎇</span>
            {branch}
          </span>
          <span className="tb-sep" />
          <label className="tb-select" title="模型">
            <select
              value={model}
              onChange={(e) => void onModelChange(e.target.value)}
              disabled={busy}
            >
              {(modelsList.length
                ? modelsList
                : [{ name: model }]
              ).map((m) => (
                <option key={m.name} value={m.name}>
                  {m.name}
                  {m.production === false ? "（桩）" : ""}
                </option>
              ))}
            </select>
          </label>
          <span className="tb-mode">{execMode.replace(/_/g, " ")}</span>
          {execMode === "full_access" && (
            <span className="tb-risk">高风险</span>
          )}
          {approval && <span className="tb-wait">待审批</span>}
        </div>
        <span className="spacer" />
        <span
          className={`status ${status === "error" ? "err" : status === "ready" || status === "busy" ? "ok" : ""}`}
        >
          {status === "error" ? statusMsg : status === "busy" ? "生成中" : "已连接"}
        </span>
        <button
          type="button"
          className="icon-btn"
          onClick={() => setTheme((t) => (t === "light" ? "dark" : "light"))}
          title="切换主题"
        >
          {theme === "light" ? "☾" : "☀"}
        </button>
        <button type="button" className="icon-btn" onClick={() => setPaletteOpen(true)} title="⌘K">
          ⌘K
        </button>
        <button
          type="button"
          className="icon-btn"
          onClick={() => setPanelOpen((v) => !v)}
          title="⌘J"
        >
          ⌫
        </button>
        <button type="button" className="ghost-btn" onClick={() => void boot()} disabled={status === "boot"}>
          重连
        </button>
      </header>

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
          {items.length === 0 && !approval && (
            <div className="hero">
              <div className="hero-mark" aria-hidden>
                NEO
              </div>
              <h1>准备就绪</h1>
              <p className="hero-sub">
                对接自家 app-server · 本地优先 · 不外传代码
              </p>
              <div className="hero-actions">
                {samplePrompts.map((s) => (
                  <button key={s} type="button" className="hero-card" onClick={() => onSample(s)}>
                    {s}
                  </button>
                ))}
              </div>
              <div className="hero-keys">
                <kbd>⌘K</kbd> 命令 · <kbd>⌘N</kbd> 新任务 · <kbd>Esc</kbd> 中断 ·{" "}
                <kbd>⇧Tab</kbd> 切换模式
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

          {grouped.map((g, i) => {
            if (g.kind === "tools") return <ToolGroup key={`t${i}`} tools={g.tools} />;
            const it = g.item;
            switch (it.type) {
              case "user":
                return (
                  <div key={i} className="msg user">
                    <div className="msg-role">
                      <span>你</span>
                      {it.text === lastUserText && !busy && !approval && (
                        <button
                          type="button"
                          className="ghost-btn edit-btn"
                          onClick={() => void onEditLastUser()}
                          title="编辑并重发（回退一轮）"
                        >
                          编辑
                        </button>
                      )}
                    </div>
                    <div className="bubble user">{it.text}</div>
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
              case "patch":
                return (
                  <div
                    key={i}
                    className="tool-card patch-card"
                    onDoubleClick={() =>
                      setDiffModal({ path: it.path, diff: it.diff })
                    }
                    title="双击全屏查看"
                  >
                    <header
                      onClick={() => setDiffModal({ path: it.path, diff: it.diff })}
                    >
                      <span className="name">⌁ {it.path}</span>
                      <span className="st">全屏</span>
                    </header>
                    <DiffView diff={it.diff} />
                  </div>
                );
              case "turn":
                return null;
              case "summary":
                return (
                  <div key={i} className="summary-bar" title="D6 轮摘要">
                    本轮 +{it.input_tokens}/−{it.output_tokens} tok ·{" "}
                    {(it.ms / 1000).toFixed(1)}s
                  </div>
                );
              case "files":
                return (
                  <div key={i} className="summary-bar files">
                    {it.files.length} 文件改动 · +
                    {it.files.reduce((s, f) => s + f.additions, 0)} −
                    {it.files.reduce((s, f) => s + f.deletions, 0)}
                  </div>
                );
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

          {stderrLines.length > 0 && (
            <details className="bubble reasoning">
              <summary>stderr（{stderrLines.length}）</summary>
              {stderrLines.join("\n")}
            </details>
          )}
        </div>
      </main>

      {panelOpen && (
        <aside className="panel">
          <div className="panel-head">
            <div className="panel-tabs" role="tablist">
              {(
                [
                  ["goal", "Goal"],
                  ["files", "文件"],
                  ["changes", "改动"],
                  ["diff", "Diff"],
                  ["wiki", "Wiki"],
                ] as const
              ).map(([id, label]) => (
                <button
                  key={id}
                  type="button"
                  role="tab"
                  aria-selected={panelTab === id}
                  className={panelTab === id ? "tab active" : "tab"}
                  onClick={() => setPanelTab(id)}
                >
                  {label}
                </button>
              ))}
            </div>
            <button type="button" className="icon-btn" onClick={() => setPanelOpen(false)}>
              ×
            </button>
          </div>
          <div className="panel-body">
            {panelTab === "goal" && (
              <section className="panel-card">
                <h3>Goal</h3>
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
                  <div className="goal-card">
                    <div className="goal-text">{goal.goal}</div>
                    <div className="muted">
                      {goal.paused ? "已暂停" : goal.stopped ? `已停：${goal.stopped}` : "运行中"}
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
                        onClick={() =>
                          void (goal.paused
                            ? goalResume(goal.goal_id)
                            : goalPause(goal.goal_id))
                        }
                      >
                        {goal.paused ? "恢复" : "暂停"}
                      </button>
                      <button type="button" onClick={() => void goalClear()}>
                        清除
                      </button>
                    </div>
                  </div>
                ) : (
                  <p className="muted">未设定目标</p>
                )}
                <div className="mode-grid" style={{ marginTop: 12 }}>
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
              </section>
            )}

            {panelTab === "files" && (
              <section className="panel-card files-card">
                <h3>工作区</h3>
                <FileTree
                  root={workspaceRoot || undefined}
                  selected={filePreview?.path ?? null}
                  onSelect={(p) => {
                    setPreviewLoading(true);
                    void readWorkspaceFile(workspaceRoot || undefined, p)
                      .then((r) => setFilePreview(r))
                      .catch((e) =>
                        setFilePreview({
                          path: p,
                          binary: false,
                          content: String(e),
                          bytes: 0,
                        }),
                      )
                      .finally(() => setPreviewLoading(false));
                  }}
                />
                {filePreview && (
                  <div className="file-preview">
                    <div className="preview-head">
                      <span className="path" title={filePreview.path}>
                        {filePreview.path}
                      </span>
                      <span className="bytes">{filePreview.bytes} B</span>
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
                      <p className="muted">二进制文件，不当作文本预览</p>
                    ) : (
                      <pre className="preview-body">
                        {filePreview.content ?? ""}
                      </pre>
                    )}
                  </div>
                )}
                <p className="muted">点文件预览 ·「加入引用」写入 @ref</p>
              </section>
            )}

            {panelTab === "changes" && (
              <section className="panel-card">
                <h3>改动文件</h3>
                {files.length === 0 ? (
                  <p className="muted">本轮尚无文件变更</p>
                ) : (
                  <ul className="file-list">
                    {files.map((f) => (
                      <li key={f.path}>
                        <span className="path" title={f.path}>
                          {f.path}
                        </span>
                        <span className="stat">
                          <span className="add">+{f.additions}</span>{" "}
                          <span className="del">−{f.deletions}</span>
                        </span>
                      </li>
                    ))}
                  </ul>
                )}
              </section>
            )}

            {panelTab === "diff" && (
              <section className="panel-card">
                <h3>Diff</h3>
                {lastPatch ? (
                  <>
                    <div className="panel-path">
                      {lastPatch.path}{" "}
                      <button
                        type="button"
                        className="ghost-btn"
                        onClick={() => setDiffModal({ ...lastPatch })}
                      >
                        全屏
                      </button>
                    </div>
                    <DiffView diff={lastPatch.diff} />
                  </>
                ) : (
                  <p className="muted">审批预览或 apply_patch 会出现在这里</p>
                )}
              </section>
            )}

            {panelTab === "wiki" && (
              <section className="panel-card wiki-card">
                <h3>Repo Wiki</h3>
                <RepoWiki root={workspaceRoot || undefined} />
              </section>
            )}

            <section className="panel-card">
              <h3>连接</h3>
              <p className="muted mono">
                {init?.server?.name ?? "neo-app-server"} · v{init?.protocol_version ?? "—"} ·{" "}
                {methods.length} methods
              </p>
              {workspaceRoot && (
                <p className="muted mono path-line" title={workspaceRoot}>
                  {workspaceRoot}
                </p>
              )}
            </section>
          </div>
        </aside>
      )}

      <div className="status-bar">
        <span className="sb-left">
          <span className="sb-item">{branch || "—"}</span>
          <span className="sb-item">{execMode.replace(/_/g, " ")}</span>
          {approval && <span className="sb-item warn">待审批</span>}
        </span>
        <span className="sb-right">
          {lastSummary && (
            <span className="sb-item">
              +{lastSummary.in}/−{lastSummary.out} tok
            </span>
          )}
          <span className="sb-item">{model}</span>
          <span className={`sb-item ${status === "error" ? "err" : "ok"}`}>
            {status === "busy" ? "运行中" : status === "error" ? "异常" : "就绪"}
          </span>
        </span>
      </div>

      <footer className="composer">
        <div className="composer-card">
          <div className="at-wrap">
            <textarea
              ref={inputRef}
              value={input}
              placeholder={
                approval
                  ? "待审批 — 输入已锁定"
                  : "描述任务，输入 @ 引用文件，或 ⌘K 命令…"
              }
              onChange={(e) => onInputChange(e.target.value)}
              onKeyDown={(e) => {
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
                if (e.key === "Escape" && !approval) {
                  e.preventDefault();
                  if (busy) void stop();
                }
              }}
              disabled={Boolean(approval) || (status !== "ready" && status !== "busy")}
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
              {approval ? "等待审批" : busy ? "生成中 · Esc 中断" : "Enter 发送"}
              {lastSummary ? ` · 上轮 +${lastSummary.in}/−${lastSummary.out}` : ""}
            </span>
            <select
              className="model-inline"
              value={model}
              disabled={busy}
              onChange={(e) => void onModelChange(e.target.value)}
              title="模型"
            >
              {(modelsList.length ? modelsList : [{ name: model }]).map((m) => (
                <option key={m.name} value={m.name}>
                  {m.name}
                </option>
              ))}
            </select>
            <button
              type="button"
              className="send-circle"
              onClick={() => void send()}
              disabled={blocked || !input.trim()}
              aria-label="发送"
            >
              {busy ? "■" : "↑"}
            </button>
            {busy && (
              <button type="button" className="ghost-btn" onClick={() => void stop()}>
                停止
              </button>
            )}
          </div>
        </div>
      </footer>
    </div>
  );
}
