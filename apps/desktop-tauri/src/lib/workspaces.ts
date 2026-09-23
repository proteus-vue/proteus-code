/**
 * 最近工作区（项目）列表 —— 仅 localStorage。
 * 会话库在 `{workspace}/.neo/sessions`，切项目 = 换目录重启 app-server。
 */

export type RecentWorkspace = {
  path: string;
  /** 目录名展示 */
  name: string;
  lastMs: number;
};

const KEY = "neo-recent-workspaces";
const MAX = 24;

export function basename(p: string): string {
  const s = p.replace(/\/+$/, "");
  const i = s.lastIndexOf("/");
  return i >= 0 ? s.slice(i + 1) : s;
}

export function loadRecentWorkspaces(): RecentWorkspace[] {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return [];
    const v = JSON.parse(raw);
    if (!Array.isArray(v)) return [];
    return v.filter(
      (x) => x && typeof x.path === "string" && x.path.length > 0,
    ).slice(0, MAX);
  } catch {
    return [];
  }
}

export function saveRecentWorkspaces(list: RecentWorkspace[]): void {
  localStorage.setItem(KEY, JSON.stringify(list.slice(0, MAX)));
}

/** 记到最前；同 path 去重 */
export function touchWorkspace(path: string): RecentWorkspace[] {
  const clean = path.replace(/\/+$/, "");
  if (!clean) return loadRecentWorkspaces();
  const rest = loadRecentWorkspaces().filter((w) => w.path !== clean);
  const next: RecentWorkspace[] = [
    { path: clean, name: basename(clean), lastMs: Date.now() },
    ...rest,
  ];
  saveRecentWorkspaces(next);
  return next;
}

export function removeWorkspace(path: string): RecentWorkspace[] {
  const next = loadRecentWorkspaces().filter((w) => w.path !== path);
  saveRecentWorkspaces(next);
  return next;
}
