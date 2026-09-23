import { useState, type MouseEvent } from "react";
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
  projectLabel: string;
  workspaceRoot: string;
  filesFoot?: string;
}) {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  /** 二次点击确认删除 —— 不用 window.confirm（Tauri 里不可靠） */
  const [confirmDelId, setConfirmDelId] = useState<string | null>(null);

  /** IA-19：标题溢出时悬停横向滚出全文 */
  const onRowEnter = (e: MouseEvent<HTMLButtonElement>) => {
    const box = e.currentTarget.querySelector<HTMLElement>(".title");
    const inner = box?.querySelector<HTMLElement>(".title-text");
    if (!box || !inner) return;
    box.classList.remove("marquee");
    // 强制 reflow 后再量，避免连续 hover 用到旧值
    void inner.offsetWidth;
    const overflow = inner.scrollWidth - box.clientWidth;
    if (overflow <= 4) return;
    const dur = Math.min(14, Math.max(3, overflow / 48));
    box.style.setProperty("--mx", `-${overflow}px`);
    box.style.setProperty("--md", `${dur}s`);
    box.classList.add("marquee");
  };

  const onRowLeave = (e: MouseEvent<HTMLButtonElement>) => {
    e.currentTarget.querySelector<HTMLElement>(".title")?.classList.remove("marquee");
  };

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

  /** IA-18：无侧栏搜索框 —— 列表始终全量；全局检索走 ⌘K */
  const filtered = threads;

  const commitRename = (id: string) => {
    const t = draft.trim();
    setEditingId(null);
    if (t) onRename(id, t);
  };

  return (
    <aside className="sidebar">
      {/* 顶栏一行：折叠 · 主按钮 · 主题 · 设置（无第二行工具） */}
      <div className="rail-head">
        <button type="button" className="icon-btn" onClick={onToggle} title="折叠侧栏 ⌘B" aria-label="折叠侧栏">
          <Icon name="chevron-left" size={16} />
        </button>
        <button type="button" className="btn-new compact" onClick={onNew} title="⌘N">
          <Icon name="plus" size={14} />
          新建任务
        </button>
        <button type="button" className="icon-btn" onClick={onTheme} title="切换主题" aria-label="切换主题">
          <Icon name={theme === "dark" ? "sun" : "moon"} size={15} />
        </button>
        <button type="button" className="icon-btn" onClick={onSettings} title="设置 ⌘," aria-label="设置">
          <Icon name="settings" size={15} />
        </button>
      </div>

      <div className="sidebar-section">
        <span className="proj-label" title={workspaceRoot}>
          <Icon name="chevron-down" size={11} className="proj-caret" />
          {projectLabel}
        </span>
      </div>

      {/* 无常驻搜索框（IA-18）：全局检索走 ⌘K；列表展示全部会话 */}
      <ul className="sidebar-list">
        {threads.length === 0 && (
          <li>
            <button type="button" className="active">
              <span className="title">当前任务</span>
            </button>
          </li>
        )}
        {filtered.length === 0 && threads.length > 0 && (
          <li className="sidebar-empty">无匹配会话</li>
        )}
        {filtered.map((t) => {
          const rel = formatRelative(t.updated_ms);
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
                onMouseEnter={onRowEnter}
                onMouseLeave={onRowLeave}
                title={[
                  t.title || t.id,
                  rel ? `更新 ${rel}` : "",
                  t.records != null && t.records > 0 ? `${t.records} 条` : "",
                  t.state ?? "",
                ]
                  .filter(Boolean)
                  .join(" · ")}
              >
                <span className="title">
                  <span className="title-text">{t.title || t.id}</span>
                </span>
                {rel && <span className="rel">{rel}</span>}
              </button>
              <span className="row-actions">
                <button
                  type="button"
                  className="row-icon"
                  aria-label="重命名"
                  onClick={(e) => {
                    e.stopPropagation();
                    setConfirmDelId(null);
                    setDraft(t.title || t.id);
                    setEditingId(t.id);
                  }}
                >
                  <Icon name="edit" size={14} />
                </button>
                <button
                  type="button"
                  className={`row-icon ${confirmDelId === t.id ? "armed" : ""}`}
                  aria-label={confirmDelId === t.id ? "再次点击删除" : "删除会话"}
                  onClick={(e) => {
                    e.stopPropagation();
                    setEditingId(null);
                    if (confirmDelId !== t.id) {
                      setConfirmDelId(t.id);
                      return;
                    }
                    setConfirmDelId(null);
                    onDelete(t.id);
                  }}
                >
                  <Icon name="trash" size={14} />
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
