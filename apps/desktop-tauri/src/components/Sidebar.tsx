import { useState } from "react";
import { formatRelative } from "../lib/time";
import type { ThreadSummary } from "../lib/protocol";
import { Icon } from "./Icon";

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
        <button type="button" className="rail-btn" onClick={onToggle} title="展开侧栏 ⌘B" aria-label="展开侧栏">
          <Icon name="menu" size={18} />
        </button>
        <button type="button" className="rail-btn" onClick={onNew} title="新建任务 ⌘N" aria-label="新建任务">
          <Icon name="plus" size={18} />
        </button>
        <button type="button" className="rail-btn" onClick={onCommand} title="命令 ⌘K" aria-label="搜索命令">
          <Icon name="search" size={18} />
        </button>
        <button type="button" className="rail-btn" onClick={onSettings} title="设置 ⌘," aria-label="设置">
          <Icon name="settings" size={18} />
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
        <button type="button" className="icon-btn" onClick={onToggle} title="折叠侧栏 ⌘B" aria-label="折叠侧栏">
          <Icon name="chevron-left" size={16} />
        </button>
        <button type="button" className="btn-new compact" onClick={onNew} title="⌘N">
          <Icon name="plus" size={14} />
          新建任务
        </button>
        <button type="button" className="icon-btn" onClick={onTheme} title="切换主题" aria-label="切换主题">
          <Icon name={theme === "dark" ? "sun" : "moon"} size={16} />
        </button>
        <button type="button" className="icon-btn" onClick={onSettings} title="设置 ⌘," aria-label="设置">
          <Icon name="settings" size={16} />
        </button>
      </div>
      <div className="sidebar-tools">
        <button type="button" className="tool-btn" onClick={onCommand} title="⌘K">
          <Icon name="search" size={16} />
          <span>搜索</span>
        </button>
        <button type="button" className="tool-btn" onClick={onNew} title="⌘N">
          <Icon name="plus" size={16} />
          <span>新建</span>
        </button>
      </div>
      <div className="sidebar-section">
        <span className="proj-label" title={workspaceRoot}>
          <Icon name="chevron-down" size={12} className="proj-caret" />
          {projectLabel}
        </span>
      </div>
      <div className="sidebar-search">
        <span className="search-lead" aria-hidden>
          <Icon name="search" size={14} />
        </span>
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
                  aria-label="重命名"
                  onClick={(e) => {
                    e.stopPropagation();
                    setDraft(t.title || t.id);
                    setEditingId(t.id);
                  }}
                >
                  <Icon name="edit" size={13} />
                </button>
                <button
                  type="button"
                  title="删除会话"
                  aria-label="删除会话"
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
                  <Icon name="trash" size={13} />
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
