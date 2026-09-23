/**
 * IA-14 Automations —— 壳侧排程提示（对齐 RpAutomation 结构，不抄实现）。
 *
 * 诚实边界：
 * - 仅本机 localStorage；桌面进程存活时才触发（不做 OS cron/launchd）
 * - 到点且 app-server ready、非 busy 时才 `turn/start`；失败记 lastError
 */

export type Automation = {
  id: string;
  title: string;
  prompt: string;
  /** 分钟间隔；0 = 不自动，仅手动 */
  everyMin: number;
  enabled: boolean;
  lastRunMs?: number;
  nextRunMs?: number;
  lastError?: string;
};

const KEY = "neo-automations-v1";

export function loadAutomations(): Automation[] {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return [];
    const v = JSON.parse(raw);
    if (!Array.isArray(v)) return [];
    return v.filter(
      (x) =>
        x &&
        typeof x.id === "string" &&
        typeof x.title === "string" &&
        typeof x.prompt === "string",
    );
  } catch {
    return [];
  }
}

export function saveAutomations(list: Automation[]): void {
  localStorage.setItem(KEY, JSON.stringify(list.slice(0, 50)));
}

export function nextDue(a: Automation, now = Date.now()): number {
  if (!a.enabled || a.everyMin <= 0) return 0;
  if (a.nextRunMs && a.nextRunMs > 0) return a.nextRunMs;
  if (a.lastRunMs) return a.lastRunMs + a.everyMin * 60_000;
  return now; // 启用后尽快跑一次
}

/** 到点列表（一次扫完，调用方负责串行发 turn） */
export function dueNow(list: Automation[], now = Date.now()): Automation[] {
  return list.filter((a) => a.enabled && a.everyMin > 0 && nextDue(a, now) <= now);
}
