import { useMemo, useState } from "react";

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
        <span>{show ? "▾" : "▸"} 思考中…</span>
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
  const ok = tools.filter((t) => t.status === "ok").length;
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
          {running ? "◌" : fail ? "●" : "✓"}
        </span>
        <span className="tl-label">
          {allDone
            ? fail
              ? `已运行 ${tools.length} 条命令 · ${fail} 失败`
              : `已运行 ${tools.length} 条命令`
            : `运行中 · ${tools.length} 步…`}
        </span>
        <span className="tl-meta">
          {ok}/{tools.length} 成功
          <span className="twist">{open ? "▾" : "▸"}</span>
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
          <span className="tl-label">{item.name}</span>
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
