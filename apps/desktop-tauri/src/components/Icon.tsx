/**
 * 自绘图标集（PRODUCT-IA §7.6 · IA-57）。
 * 两套（对齐 Codex/ZCode 导航观感）：
 * - 空心 stroke：导航/工具 chrome（menu/plus/search/…）
 * - 实心 fill：语义/标签（files/package/tab 类 + @ 文件同款文档形）
 * 单色 currentColor；禁止 emoji；不拷贝竞品。
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

/** 导航/工具 chrome → 空心 stroke */
const HOLLOW: ReadonlySet<IconName> = new Set([
  "menu",
  "plus",
  "search",
  "settings",
  "chevron-left",
  "chevron-right",
  "chevron-down",
  "sun",
  "moon",
  "close",
  "edit",
  "trash",
  "check",
  "send",
  "stop",
  "archive",
]);

/** 空心：stroke 路径（无 fill） */
const HOLLOW_PATHS: Record<IconName, ReactElement> = {
  menu: <path d="M4 7h16M4 12h16M4 17h16" />,
  plus: <path d="M12 5v14M5 12h14" />,
  search: (
    <>
      <circle cx="11" cy="11" r="6.5" />
      <path d="m16.2 16.2 4 4" />
    </>
  ),
  settings: (
    <>
      <circle cx="12" cy="12" r="3.1" />
      <path d="M12 3.2l1.1 1.9 2.2-.35.95 2 2.1.75-.35 2.2 1.6 1.55-1.6 1.55.35 2.2-2.1.75-.95 2-2.2-.35L12 20.8l-1.1-1.9-2.2.35-.95-2-2.1-.75.35-2.2L4.4 12l1.6-1.55-.35-2.2 2.1-.75.95-2 2.2.35L12 3.2z" />
    </>
  ),
  "chevron-left": <path d="M14.5 6.5 9 12l5.5 5.5" />,
  "chevron-right": <path d="M9.5 6.5 15 12l-5.5 5.5" />,
  "chevron-down": <path d="M6.5 9.5 12 15l5.5-5.5" />,
  sun: (
    <>
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2.5v2M12 19.5v2M2.5 12h2M19.5 12h2M5.05 5.05l1.4 1.4M17.55 17.55l1.4 1.4M18.95 5.05l-1.4 1.4M6.45 17.55l-1.4 1.4" />
    </>
  ),
  moon: <path d="M15.8 14.2A6.6 6.6 0 0 1 9.2 4.4 7.1 7.1 0 1 0 15.8 14.2Z" />,
  close: <path d="M7 7l10 10M17 7 7 17" />,
  edit: (
    <>
      <path d="M4.5 16.6 14.1 7l3.4 3.4-9.6 9.6H4.5v-3.4Z" />
      <path d="m13 8.1 3.4 3.4" />
    </>
  ),
  trash: (
    <>
      <path d="M5.5 7.5h13M10 7.5V5.75h4V7.5M8 7.5l.7 11.25h6.6L16 7.5" />
      <path d="M10.4 11v5.5M13.6 11v5.5" />
    </>
  ),
  check: <path d="m6 12.5 4 4L18 7.5" />,
  send: <path d="M12 19V5M6.5 11.5 12 5.5l5.5 6" />,
  stop: <rect x="7" y="7" width="10" height="10" rx="1.5" />,
  archive: (
    <>
      <path d="M4.5 5.5h15v3.5h-15zM5.75 9.5h12.5v9.75H5.75z" />
      <path d="M10 13.25h4" />
    </>
  ),
  // 占位：实心集不进此表，类型完整需要键 —— 下面 SOLID 提供
  review: <path d="M5 6h14v12H5z" />,
  terminal: <path d="M4 6h16v12H4zM8 10l2.5 2L8 14M13 14h3" />,
  browser: <path d="M4 6h16v12H4zM4 10h16" />,
  files: <path d="M7 4h7l4 4v12H7z" />,
  chat: <path d="M5 7h11v7H9l-3 2.5V14H5z" />,
  sim: <path d="M8.5 4h7v16h-7z" />,
  goal: (
    <>
      <circle cx="12" cy="12" r="7.5" />
      <circle cx="12" cy="12" r="3.5" />
    </>
  ),
  subagents: (
    <>
      <rect x="3.5" y="6" width="7" height="5.5" rx="1" />
      <rect x="13.5" y="6" width="7" height="5.5" rx="1" />
      <rect x="8.5" y="13" width="7" height="5.5" rx="1" />
    </>
  ),
  automations: (
    <>
      <circle cx="12" cy="13" r="6.5" />
      <path d="M12 10v3.5l2.5 1.5M10 3.5h4M12 3.5V6.5" />
    </>
  ),
  palette: (
    <>
      <path d="M12 4.5a7.5 7.5 0 1 0 0 15h1.2a1.8 1.8 0 0 0 1.3-3.1l-.4-.4a1.5 1.5 0 0 1 1.1-2.5H17A3.5 3.5 0 0 0 20.5 10 7.5 7.5 0 0 0 12 4.5Z" />
      <circle cx="9" cy="10" r="1" fill="currentColor" stroke="none" />
      <circle cx="12" cy="8" r="1" fill="currentColor" stroke="none" />
      <circle cx="15" cy="10" r="1" fill="currentColor" stroke="none" />
    </>
  ),
  package: <path d="M12 3.5 4.5 7.5v9L12 20.5l7.5-4v-9L12 3.5ZM4.5 7.5 12 11.5l7.5-4M12 11.5v9" />,
};

/** 实心：fill 路径（语义/标签 + 文件文档同款） */
const SOLID_PATHS: Record<IconName, ReactElement> = {
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
  package: (
    <path
      fill="currentColor"
      d="M12 2.1 3.2 6.85v10.3L12 21.9l8.8-4.35V6.85L12 2.1Zm0 2.55 6.15 3.25L12 11.1 5.85 7.9 12 4.65ZM5.25 9.55l5.9 3.15v6.55l-5.9-2.95V9.55Zm7.6 9.7V12.7l5.9-3.15v6.3l-5.9 2.95Z"
    />
  ),
  // 空心集键（类型完整；渲染时不会用到）
  menu: <path d="M4 7h16" />,
  plus: <path d="M12 5v14" />,
  search: <circle cx="11" cy="11" r="6" />,
  settings: <circle cx="12" cy="12" r="3" />,
  "chevron-left": <path d="M14 6 8 12l6 6" />,
  "chevron-right": <path d="M10 6l6 6-6 6" />,
  "chevron-down": <path d="M6 9l6 6 6-6" />,
  sun: <circle cx="12" cy="12" r="4" />,
  moon: <path d="M16 14A6.5 6.5 0 0 1 9 4a7 7 0 1 0 7 10Z" />,
  close: <path d="M7 7l10 10M17 7 7 17" />,
  edit: <path d="M5 17 15 7l3 3-10 10H5v-3Z" />,
  trash: <path d="M6 8h12l-1 11H7L6 8Zm3-2h6v2H9V6Z" />,
  check: <path d="m6 12 4 4 8-8" />,
  send: <path d="M12 19V5M6 12l6-6 6 6" />,
  stop: <rect x="7" y="7" width="10" height="10" rx="1" />,
  archive: <path d="M5 6h14v3H5zm1 4h12v9H6z" />,
};

export function Icon({
  name,
  size = 16,
  className,
  style,
  strokeWidth = 1.85,
}: {
  name: IconName;
  size?: number;
  className?: string;
  style?: CSSProperties;
  strokeWidth?: number;
}) {
  const hollow = HOLLOW.has(name);
  const paths = hollow ? HOLLOW_PATHS[name] : SOLID_PATHS[name];
  return (
    <svg
      className={className ? `icon ${className}` : "icon"}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={hollow ? strokeWidth : 0}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
      focusable="false"
      style={style}
    >
      {paths}
    </svg>
  );
}
