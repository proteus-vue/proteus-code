/**
 * Composer 上方项目选择器（ZCode 同构）。
 * 用 fixed 定位，避开 .center overflow:hidden 裁切。
 */
import { useEffect, useId, useRef, useState } from "react";
import { Icon } from "./Icon";
import type { RecentWorkspace } from "../lib/workspaces";

export function ProjectPicker({
  open,
  onClose,
  projectMode,
  projectLabel,
  workspaceRoot,
  recents,
  onSwitch,
  onNoProject,
  /** 触发芯片的视口坐标（fixed 定位） */
  anchor,
}: {
  open: boolean;
  onClose: () => void;
  projectMode: "workspace" | "none";
  projectLabel: string;
  workspaceRoot: string;
  recents: RecentWorkspace[];
  onSwitch: (path: string) => void;
  onNoProject: () => void;
  anchor: { left: number; top: number } | null;
}) {
  const id = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const [q, setQ] = useState("");
  const [addPath, setAddPath] = useState("");

  useEffect(() => {
    if (!open) return;
    setQ("");
    setAddPath("");
    const onDoc = (e: globalThis.MouseEvent) => {
      const t = e.target as HTMLElement;
      // 触发芯片在 .ctx-anchor 内 —— mousedown 不能先关，否则点一下就没了
      if (t.closest(".ctx-anchor")) return;
      if (!rootRef.current?.contains(t)) onClose();
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open, onClose]);

  if (!open || !anchor) return null;

  const filtered = recents.filter(
    (w) =>
      !q.trim() ||
      w.name.toLowerCase().includes(q.trim().toLowerCase()) ||
      w.path.toLowerCase().includes(q.trim().toLowerCase()),
  );

  const pickPath = (p: string) => {
    const clean = p.trim();
    if (!clean) return;
    onClose();
    onSwitch(clean);
  };

  // 芯片上方弹出：top = anchor.top - 菜单估高 - 8（用 bottom 钉在芯片上）
  return (
    <div
      ref={rootRef}
      id={id}
      className="proj-picker"
      role="dialog"
      aria-label="选择工作区"
      style={{
        position: "fixed",
        left: Math.max(12, anchor.left),
        // 钉在芯片上方：bottom = 视口高 - 芯片 top + 8
        bottom: Math.max(12, window.innerHeight - anchor.top + 8),
        top: "auto",
        right: "auto",
      }}
      onMouseDown={(e) => e.stopPropagation()}
    >
      <div className="proj-picker-search">
        <Icon name="search" size={14} />
        <input
          value={q}
          placeholder="搜索工作区"
          onChange={(e) => setQ(e.target.value)}
          // 不 autoFocus：避免抢焦点导致立刻触发 blur/关闭链路
        />
      </div>

      <div className="proj-picker-list" role="listbox">
        {filtered.length === 0 && !q && (
          <div className="proj-empty">暂无最近项目 · 下方粘贴路径</div>
        )}
        {q && filtered.length === 0 && (
          <div className="proj-empty">无匹配工作区</div>
        )}
        {filtered.map((w) => {
          const cur =
            projectMode === "workspace" &&
            workspaceRoot.replace(/\/+$/, "") === w.path;
          return (
            <button
              key={w.path}
              type="button"
              role="option"
              aria-selected={cur}
              className={cur ? "active" : ""}
              title={w.path}
              onClick={() => {
                if (!cur) pickPath(w.path);
                else onClose();
              }}
            >
              <Icon name="files" size={15} />
              <span className="pname">{w.name}</span>
              {cur && <Icon name="check" size={15} />}
            </button>
          );
        })}
      </div>

      <div className="proj-picker-actions">
        <button
          type="button"
          className="act"
          onClick={() => {
            const el = document.getElementById("proj-open-path") as HTMLInputElement | null;
            el?.focus();
          }}
        >
          <Icon name="plus" size={15} />
          打开路径
        </button>
        <button
          type="button"
          className="act"
          onClick={() => {
            onClose();
            onNoProject();
          }}
        >
          <Icon name="close" size={15} />
          不在项目中工作
          {projectMode === "none" && <Icon name="check" size={15} className="end" />}
        </button>
      </div>

      <div className="proj-picker-add">
        <input
          id="proj-open-path"
          value={addPath}
          placeholder="工作区绝对路径…"
          onChange={(e) => setAddPath(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") pickPath(addPath);
          }}
        />
        <button
          type="button"
          className="primary"
          disabled={!addPath.trim()}
          onClick={() => pickPath(addPath)}
        >
          打开
        </button>
      </div>

      <div className="proj-picker-foot">当前：{projectLabel}</div>
    </div>
  );
}
