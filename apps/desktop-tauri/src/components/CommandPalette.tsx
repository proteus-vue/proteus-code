import { useEffect, useMemo, useRef, useState } from "react";

export type CommandItem = {
  id: string;
  label: string;
  hint?: string;
  run: () => void | Promise<void>;
};

export function CommandPalette({
  open,
  onClose,
  commands,
}: {
  open: boolean;
  onClose: () => void;
  commands: CommandItem[];
}) {
  const [q, setQ] = useState("");
  const [idx, setIdx] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const filtered = useMemo(() => {
    const s = q.trim().toLowerCase();
    if (!s) return commands;
    return commands.filter(
      (c) =>
        c.label.toLowerCase().includes(s) ||
        (c.hint ?? "").toLowerCase().includes(s) ||
        c.id.includes(s),
    );
  }, [commands, q]);

  useEffect(() => {
    if (open) {
      setQ("");
      setIdx(0);
      // 聚焦不抢系统焦点以外的窗口 —— 仅 webview 内
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  if (!open) return null;

  const run = (c: CommandItem) => {
    onClose();
    void c.run();
  };

  return (
    <div
      className="palette-backdrop"
      onClick={onClose}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onClose();
        }
      }}
      role="presentation"
    >
      <div
        className="palette"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-label="命令中心"
      >
        <input
          ref={inputRef}
          value={q}
          placeholder="搜索命令…"
          onChange={(e) => {
            setQ(e.target.value);
            setIdx(0);
          }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") {
              e.preventDefault();
              setIdx((i) => Math.min(i + 1, filtered.length - 1));
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              setIdx((i) => Math.max(i - 1, 0));
            } else if (e.key === "Enter" && filtered[idx]) {
              e.preventDefault();
              run(filtered[idx]);
            } else if (e.key === "Escape") {
              e.preventDefault();
              onClose();
            }
          }}
        />
        <ul>
          {filtered.map((c, i) => (
            <li key={c.id}>
              <button
                type="button"
                className={i === idx ? "active" : ""}
                onMouseEnter={() => setIdx(i)}
                onClick={() => run(c)}
              >
                <span>{c.label}</span>
                {c.hint && <span className="hint">{c.hint}</span>}
              </button>
            </li>
          ))}
          {filtered.length === 0 && <li className="empty">无匹配命令</li>}
        </ul>
        <div className="palette-foot">↑↓ 选择 · Enter 执行 · Esc 关闭</div>
      </div>
    </div>
  );
}
