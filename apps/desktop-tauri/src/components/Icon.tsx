/**
 * 自绘图标集（PRODUCT-IA §7.6 · IA-55 重做）。
 * 24 viewBox · 几何更满 · 默认线宽 2 · 关键块面用 fill 加重，
 * 避免「一根细线」的简笔感。不拷贝竞品；禁止 emoji 当图标。
 */
import type { CSSProperties, ReactElement } from "react";

export type IconName =
  | "menu"
  | "plus"
  | "search"
  | "settings"
  | "chevron-left"
  | "chevron-right"
  | "chevron-down"
  | "sun"
  | "moon"
  | "close"
  | "edit"
  | "trash"
  | "check"
  | "send"
  | "stop"
  | "review"
  | "terminal"
  | "browser"
  | "files"
  | "chat"
  | "sim"
  | "goal"
  | "subagents"
  | "automations"
  | "palette"
  | "archive"
  | "package";

const PATHS: Record<IconName, ReactElement> = {
  menu: (
    <>
      <path d="M4 7.25h16v1.5H4zM4 11.25h16v1.5H4zM4 15.25h16v1.5H4z" fill="currentColor" stroke="none" />
    </>
  ),
  plus: (
    <>
      <path d="M11 4.5h2v6.5H19.5v2H13v6.5h-2V13H4.5v-2H11V4.5z" fill="currentColor" stroke="none" />
    </>
  ),
  search: (
    <>
      <circle cx="11" cy="11" r="6.25" fill="currentColor" fillOpacity="0.18" stroke="currentColor" strokeWidth="2" />
      <path d="m15.6 15.6 4.15 4.15" strokeWidth="2.25" />
      <circle cx="11" cy="11" r="2.25" fill="currentColor" stroke="none" />
    </>
  ),
  settings: (
    <>
      <circle cx="12" cy="12" r="3" fill="currentColor" stroke="none" />
      <path
        d="M12 3.2l1.2 2.1 2.4-.4 1 2.2 2.3.8-.4 2.4 1.8 1.7-1.8 1.7.4 2.4-2.3.8-1 2.2-2.4-.4L12 20.8l-1.2-2.1-2.4.4-1-2.2-2.3-.8.4-2.4L3.7 12l1.8-1.7-.4-2.4 2.3-.8 1-2.2 2.4.4L12 3.2z"
        fill="currentColor"
        fillOpacity="0.2"
        stroke="currentColor"
        strokeWidth="1.75"
        strokeLinejoin="round"
      />
      <circle cx="12" cy="12" r="1.35" fill="currentColor" stroke="none" />
    </>
  ),
  "chevron-left": <path d="M14.75 5.5 8.25 12l6.5 6.5" strokeWidth="2.25" />,
  "chevron-right": <path d="M9.25 5.5 15.75 12l-6.5 6.5" strokeWidth="2.25" />,
  "chevron-down": <path d="M5.5 9.25 12 15.75l6.5-6.5" strokeWidth="2.25" />,
  sun: (
    <>
      <circle cx="12" cy="12" r="4.25" fill="currentColor" stroke="none" />
      <path
        d="M12 2.75v2.5M12 18.75v2.5M2.75 12h2.5M18.75 12h2.5M5.4 5.4l1.75 1.75M16.85 16.85l1.75 1.75M18.6 5.4l-1.75 1.75M7.15 16.85 5.4 18.6"
        strokeWidth="2"
      />
    </>
  ),
  moon: (
    <path
      d="M16.2 14.8A6.8 6.8 0 0 1 9.2 4.2 7.3 7.3 0 1 0 16.2 14.8Z"
      fill="currentColor"
      fillOpacity="0.22"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinejoin="round"
    />
  ),
  close: (
    <>
      <path d="M6.75 6.75 17.25 17.25M17.25 6.75 6.75 17.25" strokeWidth="2.35" />
    </>
  ),
  edit: (
    <>
      <path
        d="M4.5 16.8 14.2 7.1l3.4 3.4-9.7 9.7H4.5v-3.4z"
        fill="currentColor"
        fillOpacity="0.2"
        stroke="currentColor"
        strokeWidth="1.85"
        strokeLinejoin="round"
      />
      <path d="m13.2 8.1 3.4 3.4" strokeWidth="2" />
    </>
  ),
  trash: (
    <>
      <path d="M5.5 7.5h13v1.5h-13zM9.5 7.5V5.75h5V7.5" strokeWidth="1.85" />
      <path
        d="M7.25 9h9.5l-.7 10.25H7.95L7.25 9z"
        fill="currentColor"
        fillOpacity="0.16"
        stroke="currentColor"
        strokeWidth="1.85"
        strokeLinejoin="round"
      />
      <path d="M10.4 11.5v5M13.6 11.5v5" strokeWidth="1.75" />
    </>
  ),
  check: (
    <path
      d="m5.5 12.5 4.2 4.2L18.5 7.8"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.4"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  ),
  send: (
    <>
      <path
        d="M12 4.2 4.8 19.2l7.2-3.4 7.2 3.4L12 4.2z"
        fill="currentColor"
        fillOpacity="0.22"
        stroke="currentColor"
        strokeWidth="1.85"
        strokeLinejoin="round"
      />
      <path d="M12 4.2v11.6" strokeWidth="1.75" />
    </>
  ),
  stop: (
    <rect x="6.5" y="6.5" width="11" height="11" rx="2" fill="currentColor" stroke="none" />
  ),
  review: (
    <>
      <rect
        x="4.5"
        y="5.5"
        width="15"
        height="13"
        rx="2"
        fill="currentColor"
        fillOpacity="0.16"
        stroke="currentColor"
        strokeWidth="1.85"
      />
      <path d="M7.75 10h5M7.75 13.25h8.5" strokeWidth="2" />
    </>
  ),
  terminal: (
    <>
      <rect
        x="3.5"
        y="5"
        width="17"
        height="14"
        rx="2.25"
        fill="currentColor"
        fillOpacity="0.14"
        stroke="currentColor"
        strokeWidth="1.85"
      />
      <path
        d="m7.75 10.25 2.9 2.35-2.9 2.35M13.25 15h3.5"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </>
  ),
  browser: (
    <>
      <rect
        x="3.5"
        y="5"
        width="17"
        height="14"
        rx="2.25"
        fill="currentColor"
        fillOpacity="0.12"
        stroke="currentColor"
        strokeWidth="1.85"
      />
      <path d="M3.5 9h17" strokeWidth="1.85" />
      <circle cx="7.25" cy="7" r="1" fill="#ef4444" stroke="none" />
      <circle cx="10" cy="7" r="1" fill="#f59e0b" stroke="none" />
      <circle cx="12.75" cy="7" r="1" fill="#22c55e" stroke="none" />
    </>
  ),
  files: (
    <>
      <path
        d="M6.5 3.75h7.25L18 8v12.25H6.5V3.75z"
        fill="currentColor"
        fillOpacity="0.2"
        stroke="currentColor"
        strokeWidth="1.85"
        strokeLinejoin="round"
      />
      <path d="M13.5 3.75V8H18" strokeWidth="1.85" strokeLinejoin="round" />
      <path d="M9 12.25h6M9 15.5h4.5" strokeWidth="1.85" />
    </>
  ),
  chat: (
    <>
      <path
        d="M4.5 6.75h11.5v7.5H9.25L6.5 16.75V14.25H4.5v-7.5z"
        fill="currentColor"
        fillOpacity="0.18"
        stroke="currentColor"
        strokeWidth="1.85"
        strokeLinejoin="round"
      />
      <path
        d="M9.25 10.5h9.25v6.25h-2.75l-2.25 2v-2H9.25v-6.25z"
        fill="currentColor"
        fillOpacity="0.08"
        stroke="currentColor"
        strokeWidth="1.75"
        strokeLinejoin="round"
      />
    </>
  ),
  sim: (
    <>
      <rect
        x="7.75"
        y="3.75"
        width="8.5"
        height="16.5"
        rx="2.25"
        fill="currentColor"
        fillOpacity="0.16"
        stroke="currentColor"
        strokeWidth="1.85"
      />
      <path d="M10.75 17.25h2.5" strokeWidth="2" />
      <circle cx="12" cy="7" r="1.1" fill="currentColor" stroke="none" />
    </>
  ),
  goal: (
    <>
      <circle cx="12" cy="12" r="7.75" fill="currentColor" fillOpacity="0.14" stroke="currentColor" strokeWidth="1.85" />
      <circle cx="12" cy="12" r="4" fill="currentColor" fillOpacity="0.35" stroke="currentColor" strokeWidth="1.75" />
      <circle cx="12" cy="12" r="1.5" fill="currentColor" stroke="none" />
      <path d="M12 2.75v2.25M12 19v2.25M2.75 12H5M19 12h2.25" strokeWidth="1.85" />
    </>
  ),
  subagents: (
    <>
      <rect x="3.5" y="5.5" width="7.5" height="6.5" rx="1.75" fill="currentColor" fillOpacity="0.22" stroke="currentColor" strokeWidth="1.75" />
      <rect x="13" y="5.5" width="7.5" height="6.5" rx="1.75" fill="currentColor" fillOpacity="0.22" stroke="currentColor" strokeWidth="1.75" />
      <rect x="8.25" y="13" width="7.5" height="6.5" rx="1.75" fill="currentColor" fillOpacity="0.22" stroke="currentColor" strokeWidth="1.75" />
    </>
  ),
  automations: (
    <>
      <circle cx="12" cy="13" r="6.5" fill="currentColor" fillOpacity="0.14" stroke="currentColor" strokeWidth="1.85" />
      <path d="M12 9.75v3.75l2.75 1.65" strokeWidth="2.1" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M9.75 3.5h4.5M12 3.5V7" strokeWidth="1.85" />
      <circle cx="12" cy="13" r="1.35" fill="currentColor" stroke="none" />
    </>
  ),
  palette: (
    <>
      <path
        d="M12 4.25a7.75 7.75 0 1 0 0 15.5h1.35a1.9 1.9 0 0 0 1.35-3.25l-.45-.45a1.55 1.55 0 0 1 1.1-2.6H17.2A3.55 3.55 0 0 0 20.75 11 7.75 7.75 0 0 0 12 4.25z"
        fill="currentColor"
        fillOpacity="0.16"
        stroke="currentColor"
        strokeWidth="1.85"
        strokeLinejoin="round"
      />
      <circle cx="9" cy="10.25" r="1.35" fill="#ef4444" stroke="none" />
      <circle cx="12.25" cy="8.25" r="1.35" fill="#3b82f6" stroke="none" />
      <circle cx="15.5" cy="10.5" r="1.35" fill="#22c55e" stroke="none" />
      <circle cx="10.5" cy="14" r="1.2" fill="#f59e0b" stroke="none" />
    </>
  ),
  archive: (
    <>
      <path
        d="M4.5 5.75h15v3.5h-15v-3.5z"
        fill="currentColor"
        fillOpacity="0.28"
        stroke="currentColor"
        strokeWidth="1.75"
        strokeLinejoin="round"
      />
      <path
        d="M5.75 9.75h12.5v8.5H5.75v-8.5z"
        fill="currentColor"
        fillOpacity="0.12"
        stroke="currentColor"
        strokeWidth="1.75"
        strokeLinejoin="round"
      />
      <path d="M10 13.25h4" strokeWidth="2" />
    </>
  ),
  package: (
    <>
      <path
        d="M12 3.4 4.2 7.6v8.8L12 20.6l7.8-4.2V7.6L12 3.4z"
        fill="currentColor"
        fillOpacity="0.16"
        stroke="currentColor"
        strokeWidth="1.85"
        strokeLinejoin="round"
      />
      <path d="M4.2 7.6 12 11.85l7.8-4.25M12 11.85v8.75" strokeWidth="1.85" strokeLinejoin="round" />
      <path d="M8.1 5.5 15.9 9.7" strokeWidth="1.6" opacity="0.7" />
    </>
  ),
};

export function Icon({
  name,
  size = 16,
  className,
  style,
  strokeWidth = 2,
}: {
  name: IconName;
  size?: number;
  className?: string;
  style?: CSSProperties;
  strokeWidth?: number;
}) {
  return (
    <svg
      className={className ? `icon ${className}` : "icon"}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
      focusable="false"
      style={style}
    >
      {PATHS[name]}
    </svg>
  );
}
