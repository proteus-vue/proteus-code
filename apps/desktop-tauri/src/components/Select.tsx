/**
 * 自绘下拉选择器（PRODUCT-IA §7.7）—— 禁止产品 UI 使用原生 <select>。
 * 与 palette 同 elevated 面；键盘 ↑↓/Enter/Esc。
 */
import { useEffect, useId, useRef, useState } from "react";
import { Icon } from "./Icon";

export type SelectOption = {
  value: string;
  label: string;
  hint?: string;
};

export function Select({
  value,
  options,
  onChange,
  disabled,
  title,
  className,
  ariaLabel,
}: {
  value: string;
  options: SelectOption[];
  onChange: (v: string) => void;
  disabled?: boolean;
  title?: string;
  className?: string;
  ariaLabel?: string;
}) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const [idx, setIdx] = useState(() =>
    Math.max(0, options.findIndex((o) => o.value === value)),
  );
  const rootRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLUListElement>(null);

  const current =
    options.find((o) => o.value === value) ?? options[0];

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  useEffect(() => {
    if (open) {
      const el = listRef.current?.querySelector<HTMLElement>('[data-active="1"]');
      el?.scrollIntoView({ block: "nearest" });
    }
  }, [open, idx]);

  const commit = (i: number) => {
    const o = options[i];
    if (!o) return;
    onChange(o.value);
    setOpen(false);
  };

  return (
    <div
      ref={rootRef}
      className={`ui-select ${className ?? ""} ${open ? "open" : ""} ${disabled ? "disabled" : ""}`}
      title={title}
    >
      <button
        type="button"
        className="ui-select-trigger"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        aria-controls={id}
        onClick={() => {
          if (disabled) return;
          setIdx(Math.max(0, options.findIndex((o) => o.value === value)));
          setOpen((v) => !v);
        }}
        onKeyDown={(e) => {
          if (disabled) return;
          if (e.key === "ArrowDown" || e.key === "ArrowUp" || e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            if (!open) {
              setOpen(true);
              return;
            }
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              commit(idx);
              return;
            }
            const d = e.key === "ArrowDown" ? 1 : -1;
            setIdx((i) => {
              const n = Math.min(options.length - 1, Math.max(0, i + d));
              return n;
            });
          } else if (e.key === "Escape" && open) {
            e.preventDefault();
            setOpen(false);
          }
        }}
      >
        <span className="ui-select-label">{current?.label ?? "—"}</span>
        <Icon name="chevron-down" size={14} className="ui-select-caret" />
      </button>
      {open && (
        <ul id={id} className="ui-select-menu" role="listbox" ref={listRef}>
          {options.map((o, i) => (
            <li key={o.value}>
              <button
                type="button"
                role="option"
                aria-selected={o.value === value}
                data-active={i === idx ? "1" : "0"}
                className={i === idx ? "active" : ""}
                onMouseEnter={() => setIdx(i)}
                onClick={() => commit(i)}
                onKeyDown={(e) => {
                  if (e.key === "ArrowDown") {
                    e.preventDefault();
                    setIdx((x) => Math.min(options.length - 1, x + 1));
                  } else if (e.key === "ArrowUp") {
                    e.preventDefault();
                    setIdx((x) => Math.max(0, x - 1));
                  } else if (e.key === "Enter") {
                    e.preventDefault();
                    commit(i);
                  }
                }}
              >
                <span>{o.label}</span>
                {o.hint && <span className="hint">{o.hint}</span>}
                {o.value === value && <Icon name="check" size={14} />}
              </button>
            </li>
          ))}
          {options.length === 0 && <li className="empty">无选项</li>}
        </ul>
      )}
    </div>
  );
}
