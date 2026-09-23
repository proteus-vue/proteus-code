import type { GoalSnapshot } from "../lib/protocol";

/** 右侧进度浮层（对标截图 2「进程 5/6」）：有 Goal 时挂在流区右上。 */
export function ProgressFloat({
  goal,
  busy,
  onCollapse,
}: {
  goal: GoalSnapshot | null;
  busy: boolean;
  onCollapse?: () => void;
}) {
  if (!goal) return null;
  const total = goal.subtasks.length || 1;
  const done = goal.subtasks.filter((s) => s.phase === "done").length;
  const current = goal.subtasks.find((s) => s.phase !== "done");

  return (
    <aside className="progress-float" aria-label="目标进度">
      <header className="pf-head">
        <div className="pf-title">
          <span className="pf-label">进程</span>
          <span className="pf-count">
            {done}/{goal.subtasks.length || total}
          </span>
        </div>
        <div className="pf-actions">
          <button type="button" className="icon-btn" onClick={onCollapse} title="收起">
            ×
          </button>
        </div>
      </header>

      <div className="pf-goal" title={goal.goal}>
        {goal.goal.split("\n")[0]}
      </div>

      <ul className="pf-list">
        {goal.subtasks.map((s) => {
          const state =
            s.phase === "done" ? "done" : s === current && busy ? "run" : "todo";
          return (
            <li key={s.id} className={state}>
              <span className="pf-dot" aria-hidden>
                {state === "done" ? "✓" : state === "run" ? "●" : "○"}
              </span>
              <span className="pf-text">{s.title}</span>
            </li>
          );
        })}
        {goal.subtasks.length === 0 && (
          <li className="todo">
            <span className="pf-dot">○</span>
            <span className="pf-text muted">等待拆解…</span>
          </li>
        )}
      </ul>

      <footer className="pf-foot">
        {goal.paused ? "已暂停" : goal.stopped ? `已停：${goal.stopped}` : busy ? "运行中" : "就绪"}
        {" · iter "}
        {goal.iterations}
        {goal.turns_remaining > 0 && ` · 剩余 ${goal.turns_remaining} 轮`}
      </footer>
    </aside>
  );
}
