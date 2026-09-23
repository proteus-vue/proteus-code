import { useEffect, useState } from "react";
import { DiffView } from "./DiffView";

/** D5 全屏 diff：统一 / 并排切换（v）· Esc 关闭。 */
export function DiffModal({
  path,
  diff,
  onClose,
}: {
  path: string;
  diff: string;
  onClose: () => void;
}) {
  const [mode, setMode] = useState<"unified" | "split">("split");

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onClose();
      } else if (e.key === "v" || e.key === "V") {
        e.preventDefault();
        e.stopPropagation();
        setMode((m) => (m === "split" ? "unified" : "split"));
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  return (
    <div className="diff-modal-backdrop" onClick={onClose} role="presentation">
      <div
        className="diff-modal"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-label={path}
      >
        <header className="diff-modal-head">
          <span className="path">{path}</span>
          <div className="actions">
            <button
              type="button"
              className={mode === "split" ? "primary" : "ghost-btn"}
              onClick={() => setMode("split")}
              title="v"
            >
              并排
            </button>
            <button
              type="button"
              className={mode === "unified" ? "primary" : "ghost-btn"}
              onClick={() => setMode("unified")}
              title="v"
            >
              统一
            </button>
            <button type="button" className="ghost-btn" onClick={onClose}>
              关闭 Esc
            </button>
          </div>
        </header>
        <DiffView diff={diff} mode={mode} />
        <footer className="diff-modal-foot">v 切换视图 · Esc 关闭</footer>
      </div>
    </div>
  );
}
