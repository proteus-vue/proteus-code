import { useMemo, useState } from "react";
import { Icon } from "./Icon";

/** D3：思考轨迹可折叠 + 搜索。默认折叠（规格：思考默认收起）。 */
export function ThinkingBlock({
  text,
  search,
  forceOpen,
}: {
  text: string;
  search?: string;
  forceOpen?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const chars = text.length;
  const hit = useMemo(() => {
    if (!search?.trim()) return false;
    return text.toLowerCase().includes(search.trim().toLowerCase());
  }, [search, text]);
  const show = forceOpen || hit || open;
  return (
    <div className={`thinking ${hit ? "hit" : ""}`}>
      <button
        type="button"
        className="thinking-head"
        onClick={() => setOpen((v) => !v)}
      >
        <span className="twist-lead">
          <Icon name={show ? "chevron-down" : "chevron-right"} size={12} />
        </span>{" "}
        思考中…
        <span className="meta">{chars} 字</span>
        {hit && <span className="meta hit-tag">命中</span>}
      </button>
      {show && <div className="thinking-body">{text}</div>}
    </div>
  );
}

export type ToolItem = {
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
};

/** ZCode 式工具行：查阅 · N 搜索, M 文件 · 编辑 · 运行 … */
function summarizeTools(tools: ToolItem[]): string {
  let searches = 0;
  let files = 0;
  let runs = 0;
  let edits = 0;
  let todos = 0;
  let asks = 0;
  for (const t of tools) {
    if (t.name === "apply_patch") {
      edits++;
      continue;
    }
    if (t.name === "todowrite") {
      todos++;
      continue;
    }
    if (t.name === "request_user_input") {
      asks++;
      continue;
    }
    if (t.name === "bash") {
      const cmd =
        typeof t.args === "object" && t.args
          ? String((t.args as Record<string, unknown>).cmd ?? "")
          : "";
      if (/\b(rg|grep|find|fd|ag)\b/.test(cmd)) searches++;
      else if (/\b(cat|head|tail|ls|wc|stat)\b/.test(cmd)) files++;
      else runs++;
      continue;
    }
    runs++;
  }
  const parts: string[] = [];
  if (searches || files) {
    const bits: string[] = [];
    if (searches) bits.push(`${searches} 搜索`);
    if (files) bits.push(`${files} 文件`);
    parts.push(`查阅 · ${bits.join(", ")}`);
  }
  if (edits) parts.push(`编辑 · ${edits}`);
  if (runs) parts.push(`运行 · ${runs}`);
  if (todos) parts.push(`清单 · ${todos}`);
  if (asks) parts.push(`问询 · ${asks}`);
  if (!parts.length) parts.push(`已执行 ${tools.length} 步`);
  if (failCount(tools) > 0) parts.push(`${failCount(tools)} 失败`);
  return parts.join(" · ");
}

function failCount(tools: ToolItem[]): number {
  return tools.filter((t) => t.status === "fail").length;
}

function argSummary(args: unknown): string {
  if (args == null) return "";
  if (typeof args === "string") return args.slice(0, 80);
  if (typeof args === "object") {
    const o = args as Record<string, unknown>;
    for (const k of ["cmd", "path", "command", "query", "text", "pattern"]) {
      if (typeof o[k] === "string") return String(o[k]).slice(0, 90);
    }
    try {
      return JSON.stringify(o).slice(0, 90);
    } catch {
      return "";
    }
  }
  return String(args).slice(0, 90);
}

/** D4：同轮连续工具 → 一条「已运行 N 条命令」时间线（MiMo 密度）。 */
export function ToolGroup({ tools }: { tools: ToolItem[] }) {
  const [open, setOpen] = useState(false);
  const fail = tools.filter((t) => t.status === "fail").length;
  const running = tools.some((t) => t.status === "running" || t.status === "approval");
  const allDone = !running;

  if (tools.length === 1) {
    return <ToolCard item={tools[0]} dense />;
  }

  return (
    <div className="timeline-card">
      <button
        type="button"
        className="timeline-head"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
      >
        <span className={`tl-icon ${running ? "spin" : fail ? "fail" : "ok"}`}>
          <Icon name="search" size={14} />
        </span>
        <span className="tl-label">
          {allDone
            ? summarizeTools(tools)
            : `运行中 · ${summarizeTools(tools)}`}
        </span>
        <span className="tl-meta">
          <span className="twist">
            <Icon name={open ? "chevron-down" : "chevron-right"} size={12} />
          </span>
        </span>
      </button>
      {open && (
        <ol className="timeline-rows">
          {tools.map((t, i) => (
            <li key={t.id}>
              <button
                type="button"
                className={`tl-row ${t.status}`}
                onClick={() => setOpen(true)}
              >
                <span className="n">{i + 1}</span>
                <span className="nm">{t.name}</span>
                <span className="sum">{argSummary(t.args)}</span>
                <span className="st">
                  {t.status === "running"
                    ? "…"
                    : t.status === "approval"
                      ? "审批"
                      : t.status === "ok"
                        ? "✓"
                        : "✗"}
                </span>
              </button>
              {(t.stdout || t.stderr || t.detail) && (
                <div className="tl-detail">
                  {t.detail && <div>{t.detail}</div>}
                  {t.stdout && <pre>{t.stdout}</pre>}
                  {t.stderr && <pre className="err">{t.stderr}</pre>}
                </div>
              )}
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}

export function ToolCard({
  item,
  dense,
}: {
  item: ToolItem;
  dense?: boolean;
}) {
  const [open, setOpen] = useState(!dense);
  const label =
    item.status === "running"
      ? "执行中"
      : item.status === "approval"
        ? "待审批"
        : item.status === "ok"
          ? "✓"
          : "✗";
  const st = item.status === "ok" ? "ok" : item.status === "fail" ? "fail" : "";
  const summary = argSummary(item.args);

  if (dense) {
    return (
      <div className="timeline-card single">
        <button
          type="button"
          className="timeline-head"
          onClick={() => setOpen((v) => !v)}
        >
          <span className={`tl-icon ${item.status === "running" ? "spin" : st || "ok"}`}>
            {item.status === "running" ? "◌" : item.status === "ok" ? "✓" : item.status === "approval" ? "◐" : "✗"}
          </span>
          <span className="tl-label">
            {item.name === "bash"
              ? summarizeTools([item])
              : item.name === "apply_patch"
                ? `编辑 · ${argSummary(item.args).slice(0, 60)}`
                : item.name === "todowrite"
                  ? "清单"
                  : item.name === "request_user_input"
                    ? "问询"
                    : item.name}
          </span>
          <span className="tl-meta">
            {summary && <span className="tl-sum">{summary}</span>}
            <span className={`st ${st}`}>{label}</span>
          </span>
        </button>
        {open && (item.stdout || item.stderr || item.detail || item.args != null) && (
          <div className="tl-detail">
            {item.args != null && <div className="muted">args: {summary}</div>}
            {item.detail && <div>{item.detail}</div>}
            {item.stdout && <pre>{item.stdout}</pre>}
            {item.stderr && <pre className="err">{item.stderr}</pre>}
          </div>
        )}
      </div>
    );
  }

  return (
    <div className="tool-card">
      <header onClick={() => setOpen((v) => !v)} role="button" tabIndex={0}>
        <span className="name">↳ {item.name}</span>
        {item.approvalKind && <span className="tag">[{item.approvalKind}]</span>}
        <span className={`st ${st}`}>{label}</span>
      </header>
      {open && (
        <div className="body">
          {summary && <div>args: {summary}</div>}
          {item.detail && <div>detail: {item.detail}</div>}
          {item.stdout && <div>{item.stdout}</div>}
          {item.stderr && <div>{item.stderr}</div>}
        </div>
      )}
    </div>
  );
}

export function groupTools<T>(
  items: T[],
): Array<{ kind: "item"; item: T } | { kind: "tools"; tools: ToolItem[] }> {
  const out: Array<{ kind: "item"; item: T } | { kind: "tools"; tools: ToolItem[] }> = [];
  let run: ToolItem[] = [];
  const flush = () => {
    if (run.length) {
      out.push({ kind: "tools", tools: run });
      run = [];
    }
  };
  for (const it of items) {
    if (
      it &&
      typeof it === "object" &&
      "type" in it &&
      (it as { type: string }).type === "tool"
    ) {
      run.push(it as unknown as ToolItem);
    } else {
      flush();
      out.push({ kind: "item", item: it });
    }
  }
  flush();
  return out;
}
