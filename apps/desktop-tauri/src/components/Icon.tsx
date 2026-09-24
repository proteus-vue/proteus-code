/**
 * 自绘图标集（PRODUCT-IA §7.6 · IA-55/IA-56）。
 * 单色实心：fill=currentColor · evenodd 挖孔 · 无第二色、无白描边。
 * 对齐 @ 文件引用文档图标形态。禁止 emoji；不拷贝竞品。
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
    <path
      fill="currentColor"
      d="M4 6.5h16v2.25H4V6.5Zm0 4.375h16v2.25H4v-2.25Zm0 4.375h16V17.5H4v-2.25Z"
    />
  ),
  plus: (
    <path
      fill="currentColor"
      d="M10.75 3.5h2.5v7.25H20.5v2.5h-7.25V20.5h-2.5v-7.25H3.5v-2.5h7.25V3.5Z"
    />
  ),
  search: (
    <path
      fill="currentColor"
      fillRule="evenodd"
      clipRule="evenodd"
      d="M10.75 3.5a7.25 7.25 0 1 0 4.52 12.93l3.65 3.65a1.25 1.25 0 0 0 1.77-1.77l-3.65-3.65A7.25 7.25 0 0 0 10.75 3.5Zm0 2.5a4.75 4.75 0 1 1 0 9.5 4.75 4.75 0 0 1 0-9.5Z"
    />
  ),
  settings: (
    <path
      fill="currentColor"
      fillRule="evenodd"
      clipRule="evenodd"
      d="M12 2.75 13.4 5l2.7-.45 1.15 2.5 2.6.95-.45 2.7L21 12l-1.55 1.3.45 2.7-2.6.95-1.15 2.5-2.7-.45L12 21.25l-1.4-2.25-2.7.45-1.15-2.5-2.6-.95.45-2.7L3 12l1.55-1.3-.45-2.7 2.6-.95L7.85 4.55l2.7.45L12 2.75Zm0 6.5a2.75 2.75 0 1 0 0 5.5 2.75 2.75 0 0 0 0-5.5Z"
    />
  ),
  "chevron-left": (
    <path
      fill="currentColor"
      d="M14.9 5.1 8 12l6.9 6.9 1.8-1.8L11.6 12l5.1-5.1-1.8-1.8Z"
    />
  ),
  "chevron-right": (
    <path
      fill="currentColor"
      d="M9.1 5.1 16 12l-6.9 6.9-1.8-1.8L12.4 12l-5.1-5.1 1.8-1.8Z"
    />
  ),
  "chevron-down": (
    <path
      fill="currentColor"
      d="M5.1 9.1 12 16l6.9-6.9-1.8-1.8L12 12.4l-5.1-5.1-1.8 1.8Z"
    />
  ),
  sun: (
    <path
      fill="currentColor"
      fillRule="evenodd"
      clipRule="evenodd"
      d="M12 7.25a4.75 4.75 0 1 0 0 9.5 4.75 4.75 0 0 0 0-9.5Zm-1.1-7.5h2.2v3.4h-2.2V-.25Zm0 17.6h2.2v3.4h-2.2v-3.4ZM.25 10.9h3.4v2.2H.25v-2.2Zm17.6 0h3.4v2.2h-3.4v-2.2ZM4.05 5.4l1.55-1.55 2.4 2.4-1.55 1.55-2.4-2.4Zm12 12 1.55-1.55 2.4 2.4-1.55 1.55-2.4-2.4ZM19.95 5.4l-1.55-1.55-2.4 2.4 1.55 1.55 2.4-2.4ZM5.6 17.4l-1.55-1.55-2.4 2.4L3.2 19.8l2.4-2.4Z"
    />
  ),
  moon: (
    <path
      fill="currentColor"
      d="M16.4 14.6A7.1 7.1 0 0 1 9.4 3.9a7.6 7.6 0 1 0 7 10.7Z"
    />
  ),
  close: (
    <path
      fill="currentColor"
      d="M6.4 5 5 6.4 10.6 12 5 17.6 6.4 19 12 13.4 17.6 19 19 17.6 13.4 12 19 6.4 17.6 5 12 10.6 6.4 5Z"
    />
  ),
  edit: (
    <path
      fill="currentColor"
      d="M4.2 16.55 14.05 6.7l3.55 3.55L7.75 20.1H4.2v-3.55Zm10.05-9.85 1.55-1.55 3.55 3.55-1.55 1.55-3.55-3.55Z"
    />
  ),
  trash: (
    <path
      fill="currentColor"
      fillRule="evenodd"
      clipRule="evenodd"
      d="M9.25 4.5h5.5v1.75H17V8H7V6.25h2.25V4.5ZM5.75 8h12.5l-.85 11.5a1.75 1.75 0 0 1-1.75 1.6H8.35a1.75 1.75 0 0 1-1.75-1.6L5.75 8Zm3.5 2.75v6.5h1.5v-6.5H9.25Zm4 0v6.5h1.5v-6.5h-1.5Z"
    />
  ),
  check: (
    <path
      fill="currentColor"
      d="M9.4 16.6 4.8 12l1.7-1.7 2.9 2.9 7.1-7.1L18.2 7.8 9.4 16.6Z"
    />
  ),
  send: (
    <path
      fill="currentColor"
      d="M12 3.2 3.8 20.2l8.2-3.8 8.2 3.8L12 3.2Zm0 3.35 5.35 11.15L12 15.55 6.65 17.7 12 6.55Z"
    />
  ),
  stop: (
    <rect x="6.25" y="6.25" width="11.5" height="11.5" rx="2.25" fill="currentColor" />
  ),
  review: (
    <path
      fill="currentColor"
      fillRule="evenodd"
      clipRule="evenodd"
      d="M6.25 3.5h11.5A1.75 1.75 0 0 1 19.5 5.25v13.5A1.75 1.75 0 0 1 17.75 20.5H6.25A1.75 1.75 0 0 1 4.5 18.75V5.25A1.75 1.75 0 0 1 6.25 3.5Zm1.5 4.25v1.5h8v-1.5h-8Zm0 4.25v1.5h9.5v-1.5H7.75Zm0 4.25v1.5h6v-1.5h-6Z"
    />
  ),
  terminal: (
    <path
      fill="currentColor"
      fillRule="evenodd"
      clipRule="evenodd"
      d="M5.25 4.5h13.5A1.75 1.75 0 0 1 20.5 6.25v11.5A1.75 1.75 0 0 1 18.75 19.5H5.25A1.75 1.75 0 0 1 3.5 17.75V6.25A1.75 1.75 0 0 1 5.25 4.5Zm2.75 4.1 3.1 2.55-3.1 2.55v-2h5.1v-1.6H8V8.6Zm6.4 4.9h3.35v1.6H14.4v-1.6Z"
    />
  ),
  browser: (
    <path
      fill="currentColor"
      fillRule="evenodd"
      clipRule="evenodd"
      d="M5.25 4.5h13.5A1.75 1.75 0 0 1 20.5 6.25v11.5A1.75 1.75 0 0 1 18.75 19.5H5.25A1.75 1.75 0 0 1 3.5 17.75V6.25A1.75 1.75 0 0 1 5.25 4.5Zm0 4.5v1.6h13.5V9H5.25Zm1.6 3.35a1.1 1.1 0 1 0 0 2.2 1.1 1.1 0 0 0 0-2.2Zm3.1 0a1.1 1.1 0 1 0 0 2.2 1.1 1.1 0 0 0 0-2.2Zm3.1 0a1.1 1.1 0 1 0 0 2.2 1.1 1.1 0 0 0 0-2.2Z"
    />
  ),
  files: (
    <path
      fill="currentColor"
      fillRule="evenodd"
      clipRule="evenodd"
      d="M6.25 3.25A1.75 1.75 0 0 1 8 1.5h5.35c.46 0 .9.18 1.23.51l5.15 5.15c.33.33.51.77.51 1.23V20.75A1.75 1.75 0 0 1 18.5 22.5h-10.5a1.75 1.75 0 0 1-1.75-1.75V3.25Zm7.35 2.1v3.4c0 .47.38.85.85.85h3.4l-4.25-4.25ZM8.75 12.4h6.5v1.5h-6.5v-1.5Zm0 3.25h4.5v1.5h-4.5v-1.5Z"
    />
  ),
  chat: (
    <path
      fill="currentColor"
      fillRule="evenodd"
      clipRule="evenodd"
      d="M5.25 4.75h11.5A1.75 1.75 0 0 1 18.5 6.5v7A1.75 1.75 0 0 1 16.75 15.25H10l-3.25 3v-3H5.25A1.75 1.75 0 0 1 3.5 13.5v-7A1.75 1.75 0 0 1 5.25 4.75Zm1.5 3v1.5h8.5v-1.5H6.75Zm0 3.25v1.5h6.5v-1.5h-6.5ZM10 15.25h7.25A.25.25 0 0 0 17.5 15v-1.4l1.75 1.55a.75.75 0 0 1-.5 1.3H10v-1.45Z"
    />
  ),
  sim: (
    <path
      fill="currentColor"
      fillRule="evenodd"
      clipRule="evenodd"
      d="M8.75 2.5h6.5A2.25 2.25 0 0 1 17.5 4.75v14.5A2.25 2.25 0 0 1 15.25 21.5h-6.5A2.25 2.25 0 0 1 6.5 19.25V4.75A2.25 2.25 0 0 1 8.75 2.5Zm1.5 2.5v1.5h3.5V5h-3.5Zm.75 9.75h2v1.5h-2v-1.5Z"
    />
  ),
  goal: (
    <path
      fill="currentColor"
      fillRule="evenodd"
      clipRule="evenodd"
      d="M12 3.25a8.75 8.75 0 1 0 0 17.5 8.75 8.75 0 0 0 0-17.5Zm0 3a5.75 5.75 0 1 0 0 11.5 5.75 5.75 0 0 0 0-11.5Zm0 3.25a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5Z"
    />
  ),
  subagents: (
    <path
      fill="currentColor"
      d="M3.75 5h7.5A1.5 1.5 0 0 1 12.75 6.5v4A1.5 1.5 0 0 1 11.25 12h-7.5A1.5 1.5 0 0 1 2.25 10.5v-4A1.5 1.5 0 0 1 3.75 5Zm10 0h7.5A1.5 1.5 0 0 1 22.75 6.5v4A1.5 1.5 0 0 1 21.25 12h-7.5A1.5 1.5 0 0 1 12.25 10.5v-4A1.5 1.5 0 0 1 13.75 5ZM8.75 13h7.5A1.5 1.5 0 0 1 17.75 14.5v4A1.5 1.5 0 0 1 16.25 20h-7.5A1.5 1.5 0 0 1 7.25 18.5v-4A1.5 1.5 0 0 1 8.75 13Z"
    />
  ),
  automations: (
    <path
      fill="currentColor"
      fillRule="evenodd"
      clipRule="evenodd"
      d="M12 5.75a7.25 7.25 0 1 0 0 14.5 7.25 7.25 0 0 0 0-14.5Zm.9 3.4v3.95l2.6 1.55-.75 1.3-3.35-2V9.15h1.5ZM9.75 1.5h4.5v2.25h-4.5V1.5ZM12 3.75V7"
    />
  ),
  palette: (
    <path
      fill="currentColor"
      d="M12 3.25a8.75 8.75 0 0 0 0 17.5h1.55c1.1 0 2-.9 2-2 0-.55-.22-1.05-.58-1.42-.35-.36-.57-.85-.57-1.4 0-1.1.9-2 2-2h1.9A4.35 4.35 0 0 0 22.75 11 8.75 8.75 0 0 0 12 3.25Zm-3.2 7.4a1.45 1.45 0 1 1 0 2.9 1.45 1.45 0 0 1 0-2.9Zm3.3-2.3a1.45 1.45 0 1 1 0 2.9 1.45 1.45 0 0 1 0-2.9Zm3.4 1.15a1.45 1.45 0 1 1 0 2.9 1.45 1.45 0 0 1 0-2.9Zm-2.7 4.55a1.3 1.3 0 1 1 0 2.6 1.3 1.3 0 0 1 0-2.6Z"
    />
  ),
  archive: (
    <path
      fill="currentColor"
      fillRule="evenodd"
      clipRule="evenodd"
      d="M4.5 3.75h15A1.5 1.5 0 0 1 21 5.25v3A1.5 1.5 0 0 1 19.5 9.75h-15A1.5 1.5 0 0 1 3 8.25v-3A1.5 1.5 0 0 1 4.5 3.75Zm1.25 6h12.5v9A1.5 1.5 0 0 1 16.75 20.25h-9.5A1.5 1.5 0 0 1 5.75 18.75v-9Zm4.25 3.1h4v1.5h-4v-1.5Z"
    />
  ),
  package: (
    <path
      fill="currentColor"
      d="M12 2.1 3.2 6.85v10.3L12 21.9l8.8-4.35V6.85L12 2.1Zm0 2.55 6.15 3.25L12 11.1 5.85 7.9 12 4.65ZM5.25 9.55l5.9 3.15v6.55l-5.9-2.95V9.55Zm7.6 9.7V12.7l5.9-3.15v6.3l-5.9 2.95Z"
    />
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
