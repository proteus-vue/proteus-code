import { useEffect, useMemo, useRef } from "react";
import type { ExecMode } from "../lib/rpc";

const EXEC_MODES: { id: ExecMode; label: string }[] = [
  { id: "plan", label: "Plan" },
  { id: "confirm_before", label: "确认" },
  { id: "default", label: "Default" },
  { id: "auto_edit", label: "自动编辑" },
  { id: "full_access", label: "完全访问" },
];

/** ⌘, 设置页（PRODUCT-IA §5 / IA-13）—— 模态，不改三区结构。 */
export function SettingsModal({
  open,
  onClose,
  theme,
  onTheme,
  model,
  models,
  onModel,
  execMode,
  onMode,
  protocolVersion,
  methodCount,
  workspaceRoot,
}: {
  open: boolean;
  onClose: () => void;
  theme: "light" | "dark";
  onTheme: (t: "light" | "dark") => void;
  model: string;
  models: { name: string; description?: string }[];
  onModel: (name: string) => void;
  execMode: ExecMode;
  onMode: (m: ExecMode) => void;
  protocolVersion: number | string;
  methodCount: number;
  workspaceRoot: string;
}) {
  const closeRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (open) requestAnimationFrame(() => closeRef.current?.focus());
  }, [open]);

  const shortcuts = useMemo(
    () =>
      [
        ["⌘N", "新建会话"],
        ["⌘K / ⌘P", "命令面板"],
        ["⌘B", "折叠/展开侧栏"],
        ["⌘J", "折叠/展开右栏"],
        ["⌘L", "聚焦输入"],
        ["⌘,", "设置"],
        ["⇧Tab", "循环执行模式"],
        ["Esc", "中断生成"],
      ] as const,
    [],
  );

  if (!open) return null;

  return (
    <div
      className="palette-backdrop settings-backdrop"
      onClick={onClose}
      role="presentation"
    >
      <div
        className="settings-modal"
        role="dialog"
        aria-label="设置"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="settings-head">
          <h2>设置</h2>
          <button
            ref={closeRef}
            type="button"
            className="icon-btn"
            onClick={onClose}
            title="关闭 Esc / ⌘,"
          >
            ×
          </button>
        </header>

        <div className="settings-body">
          <section className="settings-section">
            <h3>外观</h3>
            <div className="settings-row">
              <span>主题</span>
              <div className="seg">
                <button
                  type="button"
                  className={theme === "light" ? "active" : ""}
                  onClick={() => onTheme("light")}
                >
                  浅色
                </button>
                <button
                  type="button"
                  className={theme === "dark" ? "active" : ""}
                  onClick={() => onTheme("dark")}
                >
                  深色
                </button>
              </div>
            </div>
          </section>

          <section className="settings-section">
            <h3>会话</h3>
            <div className="settings-row">
              <span>模型</span>
              <select
                value={model}
                onChange={(e) => onModel(e.target.value)}
              >
                {(models.length ? models : [{ name: model }]).map((m) => (
                  <option key={m.name} value={m.name}>
                    {m.name}
                    {m.description ? ` — ${m.description}` : ""}
                  </option>
                ))}
              </select>
            </div>
            <div className="settings-row">
              <span>执行模式</span>
              <select
                value={execMode}
                onChange={(e) => onMode(e.target.value as ExecMode)}
              >
                {EXEC_MODES.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.label}
                  </option>
                ))}
              </select>
            </div>
            <div className="settings-row">
              <span>工作区</span>
              <code className="settings-path" title={workspaceRoot}>
                {workspaceRoot || "—"}
              </code>
            </div>
          </section>

          <section className="settings-section">
            <h3>快捷键</h3>
            <ul className="settings-keys">
              {shortcuts.map(([k, label]) => (
                <li key={k}>
                  <kbd>{k}</kbd>
                  <span>{label}</span>
                </li>
              ))}
            </ul>
          </section>

          <section className="settings-section">
            <h3>关于</h3>
            <p className="muted">
              NEO Desktop · 本地 app-server · 协议 v{protocolVersion} ·{" "}
              {methodCount} methods · 不上传代码
            </p>
          </section>
        </div>

        <footer className="settings-foot">
          <span className="muted">设置仅存本机（主题 localStorage；模型/模式走 session/configure）</span>
          <button type="button" className="primary" onClick={onClose}>
            完成
          </button>
        </footer>
      </div>
    </div>
  );
}
