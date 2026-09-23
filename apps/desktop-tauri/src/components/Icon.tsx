/**
 * 自绘线性图标集（PRODUCT-IA §7.6）。
 * 24 viewBox · stroke=currentColor · 1.75 · round cap。
 * 不拷贝竞品资源；emoji/系统符号不得当图标用。
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
  | "palette";

const PATHS: Record<IconName, ReactElement> = {
  menu: <path d="M4 7h16M4 12h16M4 17h16" />,
  plus: <path d="M12 5v14M5 12h14" />,
  search: (
    <>
      <circle cx="11" cy="11" r="6.5" />
      <path d="m16 16 3.5 3.5" />
    </>
  ),
  settings: (
    <>
      <circle cx="12" cy="12" r="3" />
      <path d="M12 3.5v2M12 18.5v2M3.5 12h2M18.5 12h2M6 6l1.4 1.4M16.6 16.6 18 18M18 6l-1.4 1.4M7.4 16.6 6 18" />
    </>
  ),
  "chevron-left": <path d="M14.5 6.5 9 12l5.5 5.5" />,
  "chevron-right": <path d="M9.5 6.5 15 12l-5.5 5.5" />,
  "chevron-down": <path d="M6.5 9.5 12 15l5.5-5.5" />,
  sun: (
    <>
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2.5v2M12 19.5v2M2.5 12h2M19.5 12h2M5 5l1.4 1.4M17.6 17.6 19 19M19 5l-1.4 1.4M6.4 17.6 5 19" />
    </>
  ),
  moon: <path d="M15.5 14.5A6.5 6.5 0 0 1 9.5 4 7 7 0 1 0 15.5 14.5Z" />,
  close: <path d="M7 7l10 10M17 7 7 17" />,
  edit: (
    <>
      <path d="M4 16.5 14.5 6l3.5 3.5L7.5 20H4v-3.5Z" />
      <path d="m13 7.5 3.5 3.5" />
    </>
  ),
  trash: (
    <>
      <path d="M5 7h14M10 7V5h4v2M8 7l.5 12h7L16 7" />
    </>
  ),
  check: <path d="m6 12.5 4 4L18 8" />,
  send: <path d="M12 19V5M6 11l6-6 6 6" />,
  stop: <rect x="7" y="7" width="10" height="10" rx="1.5" />,
  review: (
    <>
      <path d="M5 6.5h14v11H5z" />
      <path d="M8 10h5M8 13h8" />
    </>
  ),
  terminal: (
    <>
      <rect x="4" y="5" width="16" height="14" rx="2" />
      <path d="m8 10 2.5 2L8 14M12.5 14H16" />
    </>
  ),
  browser: (
    <>
      <rect x="4" y="5" width="16" height="14" rx="2" />
      <path d="M4 9h16" />
      <circle cx="7.5" cy="7" r="0.6" fill="currentColor" stroke="none" />
      <circle cx="9.5" cy="7" r="0.6" fill="currentColor" stroke="none" />
    </>
  ),
  files: (
    <>
      <path d="M7 4.5h7l3 3V19.5H7z" />
      <path d="M14 4.5v3h3" />
    </>
  ),
  chat: (
    <>
      <path d="M5 7.5h10.5v7H9l-2.5 2.5V14.5H5z" />
      <path d="M9 10.5h8.5v6H16l2 2v-2" />
    </>
  ),
  sim: (
    <>
      <rect x="8" y="4.5" width="8" height="15" rx="2" />
      <path d="M11 17h2" />
    </>
  ),
  goal: (
    <>
      <circle cx="12" cy="12" r="7.5" />
      <circle cx="12" cy="12" r="3.5" />
      <path d="M12 4.5v3M12 16.5v3" />
    </>
  ),
  subagents: (
    <>
      <rect x="4" y="6" width="7" height="6" rx="1.5" />
      <rect x="13" y="6" width="7" height="6" rx="1.5" />
      <rect x="8.5" y="14" width="7" height="6" rx="1.5" />
    </>
  ),
  automations: (
    <>
      <circle cx="12" cy="13" r="6.5" />
      <path d="M12 10v3.5l2.5 1.5M10 3.5h4M12 3.5V6" />
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
};

export function Icon({
  name,
  size = 16,
  className,
  style,
  strokeWidth = 1.75,
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
