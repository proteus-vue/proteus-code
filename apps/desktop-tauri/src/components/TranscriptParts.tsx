import { useMemo, useState } from "react";

/** D3：思考轨迹可折叠 + 搜索（大小写不敏感，只搜思考块）。 */
export function ThinkingBlock({
  text,
  search,
  forceOpen,
}: {
  text: string;
  search?: string;
  forceOpen?: boolean;
}) {
  const [open, setOpen] = useState(true);
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
        <span>{show ? "▾" : "▸"} 思考</span>
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

/** D4：同轮连续工具折叠成组。 */
export function ToolGroup({ tools }: { tools: ToolItem[] }) {
  const [open, setOpen] = useState(false);
  const ok = tools.filter((t) => t.status === "ok").length;
  const fail = tools.filter((t) => t.status === "fail").length;
  const running = tools.some((t) => t.status === "running" || t.status === "approval");
  const icon = running ? "⏳" : fail ? "✗" : "✓";
  if (tools.length === 1) return <ToolCard item={tools[0]} />;
  return (
    <div className="tool-card">
      <header onClick={() => setOpen((v) => !v)} role="button" tabIndex={0}>
        <span className="name">
          {icon} {tools.length} 次调用
        </span>
        <span className="st">
          {ok} ok{fail ? ` · ${fail} fail` : ""}
        </span>
      </header>
      {open && (
        <div className="tool-group-body">
          {tools.map((t) => (
            <ToolCard key={t.id} item={t} />
          ))}
        </div>
      )}
    </div>
  );
}

export function ToolCard({ item }: { item: ToolItem }) {
  const [open, setOpen] = useState(false);
  const label =
    item.status === "running"
      ? "执行中…"
      : item.status === "approval"
        ? "待审批"
        : `exit ${item.exitCode ?? (item.status === "ok" ? 0 : -1)}`;
  const st = item.status === "ok" ? "ok" : item.status === "fail" ? "fail" : "";
  const args =
    item.args && typeof item.args === "object"
      ? JSON.stringify(item.args, null, 2)
      : String(item.args ?? "");
  return (
    <div className="tool-card">
      <header onClick={() => setOpen((v) => !v)} role="button" tabIndex={0}>
        <span className="name">↳ {item.name}</span>
        {item.approvalKind && <span className="tag">[{item.approvalKind}]</span>}
        <span className={`st ${st}`}>{label}</span>
      </header>
      {open && (
        <div className="body">
          {args && <div>args: {args}</div>}
          {item.detail && <div>detail: {item.detail}</div>}
          {item.stdout && <div>stdout:\n{item.stdout}</div>}
          {item.stderr && <div>stderr:\n{item.stderr}</div>}
          {item.truncated && <div>[truncated]</div>}
        </div>
      )}
    </div>
  );
}

/** 把连续 tool 合成组（非 tool 打断）。 */
export function groupTools<T>(items: T[]): Array<{ kind: "item"; item: T } | { kind: "tools"; tools: ToolItem[] }> {
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
