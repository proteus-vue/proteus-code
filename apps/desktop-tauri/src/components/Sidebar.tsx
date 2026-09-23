import { useState } from "react";
import { formatRelative } from "../lib/time";
import type { ThreadSummary } from "../lib/protocol";

export function Sidebar({
  open,
  onToggle,
  onNew,
  onCommand,
  onTheme,
  onSettings,
  theme,
  threads,
  activeId,
  onResume,
  onRename,
  onDelete,
  query,
  onQuery,
  contentHits,
  projectLabel,
  workspaceRoot,
  filesFoot,
}: {
  open: boolean;
  onToggle: () => void;
  onNew: () => void;
  onCommand: () => void;
  onTheme: () => void;
  onSettings: () => void;
  theme: "light" | "dark";
  threads: ThreadSummary[];
  activeId: string | null;
  onResume: (id: string) => void;
  onRename: (id: string, title: string) => void;
  onDelete: (id: string) => void;
  query: string;
  onQuery: (q: string) => void;
  /** 内容级搜索命中：threadId → 摘要片段（来自 thread/history） */
  contentHits?: Map<string, string>;
  projectLabel: string;
  workspaceRoot: string;
  filesFoot?: string;
}) {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draft, setDraft] = useState("");

  if (!open) {
    return (
      <nav className="rail left-rail" aria-label="侧栏折叠">
        <button type="button" className="rail-btn" onClick={onToggle} title="展开侧栏 ⌘B">
          ☰
        </button>
        <button type="button" className="rail-btn" onClick={onNew} title="新建任务 ⌘N">
          ＋
        </button>
        <button type="button" className="rail-btn" onClick={onCommand} title="命令 ⌘K">
          ⌕
        </button>
        <button type="button" className="rail-btn" onClick={onSettings} title="设置 ⌘,">
          ⚙
        </button>
      </nav>
    );
  }

  const q = query.trim().toLowerCase();
  const filtered = threads.filter((t) => {
    if (!q) return true;
    if ((t.title ?? "").toLowerCase().includes(q) || t.id.toLowerCase().includes(q)) {
      return true;
    }
    // 内容命中（thread/history 懒加载）
    const hit = contentHits?.get(t.id);
    return Boolean(hit && hit.toLowerCase().includes(q));
  });

  const commitRename = (id: string) => {
    const t = draft.trim();
    setEditingId(null);
    if (t) onRename(id, t);
  };

  return (
    <aside className="sidebar">
      <div className="rail-head">
        <button type="button" className="icon-btn" onClick={onToggle} title="折叠侧栏 ⌘B">
          ◀
        </button>
        <button type="button" className="btn-new compact" onClick={onNew} title="⌘N">
          ＋ 新建任务
        </button>
        <button type="button" className="icon-btn" onClick={onTheme} title="切换主题">
          {theme === "dark" ? "☀" : "☾"}
        </button>
        <button type="button" className="icon-btn" onClick={onSettings} title="设置 ⌘,">
          ⚙
        </button>
      </div>
      <div className="sidebar-tools">
        <button type="button" className="tool-btn" onClick={onCommand} title="⌘K">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><circle cx="11" cy="11" r="7"/><path d="m20 20-3-3"/></svg>
          <span>搜索</span>
        </button>
        <button type="button" className="tool-btn" onClick={onNew} title="⌘N">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M12 5v14M5 12h14"/></svg>
          <span>新建</span>
        </button>
      </div>
      <div className="sidebar-section">
        <span className="proj-label" title={workspaceRoot}>
          ▸ {projectLabel}
        </span>
      </div>
      <div className="sidebar-search">
        <input
          value={query}
          placeholder="搜索标题或内容…"
          onChange={(e) => onQuery(e.target.value)}
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
        {filtered.length === 0 && threads.length > 0 && (
          <li className="sidebar-empty">无匹配会话</li>
        )}
        {filtered.map((t) => {
          const rel = formatRelative(t.updated_ms);
          const hit = contentHits?.get(t.id);
          const showHit = Boolean(
            q && hit && !(t.title ?? "").toLowerCase().includes(q) && !t.id.toLowerCase().includes(q),
          );
          if (editingId === t.id) {
            return (
              <li key={t.id}>
                <div className="row-edit">
                  <input
                    autoFocus
                    value={draft}
                    onChange={(e) => setDraft(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") commitRename(t.id);
                      if (e.key === "Escape") setEditingId(null);
                    }}
                    onBlur={() => commitRename(t.id)}
                    maxLength={48}
                  />
                </div>
              </li>
            );
          }
          return (
            <li key={t.id} className="row-wrap">
              <button
                type="button"
                className={activeId === t.id ? "active" : ""}
                onClick={() => onResume(t.id)}
                onDoubleClick={() => {
                  setDraft(t.title || t.id);
                  setEditingId(t.id);
                }}
              >
                <span className="row1">
                  <span className={`dot st-${t.state ?? "empty"}`} />
                  <span className={`title ${t.has_title === false ? "faded" : ""}`}>
                    {t.title || t.id}
                  </span>
                  {t.additions != null && t.deletions != null && t.additions + t.deletions > 0 && (
                    <span className="delta">
                      +{t.additions} −{t.deletions}
                    </span>
                  )}
                  {rel && <span className="rel">{rel}</span>}
                </span>
                <span className="meta">
                  {t.state ?? "empty"}
                  {t.records != null && t.records > 0 && ` · ${t.records} 条`}
                  {showHit && hit ? ` · ${hit.slice(0, 48)}` : ""}
                </span>
              </button>
              <span className="row-actions">
                <button
                  type="button"
                  title="重命名"
                  onClick={(e) => {
                    e.stopPropagation();
                    setDraft(t.title || t.id);
                    setEditingId(t.id);
                  }}
                >
                  ✎
                </button>
                <button
                  type="button"
                  title="删除会话"
                  onClick={(e) => {
                    e.stopPropagation();
                    if (t.id === activeId) {
                      window.alert("不能删除当前会话 — 先切换到别的会话再删。");
                      return;
                    }
                    if (window.confirm(`删除会话「${t.title || t.id}」？此操作不可撤销。`)) {
                      onDelete(t.id);
                    }
                  }}
                >
                  ×
                </button>
              </span>
            </li>
          );
        })}
      </ul>
      {filesFoot && <div className="sidebar-foot">{filesFoot}</div>}
    </aside>
  );
}
