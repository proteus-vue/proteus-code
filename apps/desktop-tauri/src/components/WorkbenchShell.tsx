import { useState } from "react";

export type WorkbenchId =
  | "review"
  | "terminal"
  | "files"
  | "browser"
  | "chat"
  | "sim"
  | "goal"
  | "subagents"
  | "automations";

export const WB_LABELS: Record<WorkbenchId, string> = {
  review: "审查",
  terminal: "终端",
  files: "文件",
  browser: "浏览器",
  chat: "侧边聊天",
  sim: "模拟器",
  goal: "Goal",
  subagents: "子智能体",
  automations: "Automations",
};

export const WB_MENU: { id: WorkbenchId; key: string; icon: string }[] = [
  { id: "review", key: "⇧⌘G", icon: "▦" },
  { id: "terminal", key: "⌘`", icon: ">_" },
  { id: "browser", key: "⌘T", icon: "◎" },
  { id: "files", key: "⌘P", icon: "▤" },
  { id: "chat", key: "⌥⌘S", icon: "💬" },
  { id: "sim", key: "", icon: "▣" },
  { id: "goal", key: "", icon: "◎" },
  { id: "subagents", key: "", icon: "⧉" },
  { id: "automations", key: "", icon: "⏲" },
];

/** 浏览器式标签条 + 内容槽（PRODUCT-IA §2.3） */
export function WorkbenchShell({
  openTabs,
  active,
  onActive,
  onCloseTab,
  onOpen,
  onClosePanel,
  children,
}: {
  openTabs: WorkbenchId[];
  active: WorkbenchId;
  onActive: (id: WorkbenchId) => void;
  onCloseTab: (id: WorkbenchId) => void;
  onOpen: (id: WorkbenchId) => void;
  onClosePanel: () => void;
  children: React.ReactNode;
}) {
  const [menu, setMenu] = useState(false);

  return (
    <aside className="panel workbench">
      <div className="browser-tabs" role="tablist">
        {openTabs.map((id) => (
          <div
            key={id}
            className={`btab ${active === id ? "active" : ""}`}
            role="tab"
            aria-selected={active === id}
          >
            <button type="button" className="btab-label" onClick={() => onActive(id)}>
              {WB_LABELS[id]}
            </button>
            <button
              type="button"
              className="btab-close"
              title="关闭标签"
              onClick={() => onCloseTab(id)}
            >
              ×
            </button>
          </div>
        ))}
        <div className={`btab-add ${menu ? "open" : ""}`}>
          <button
            type="button"
            className={`btab-plus ${menu ? "active" : ""}`}
            title="打开标签页"
            aria-expanded={menu}
            onClick={(e) => {
              e.stopPropagation();
              setMenu((v) => !v);
            }}
          >
            +
          </button>
          {menu && (
            <ul className="btab-menu" role="menu" onClick={(e) => e.stopPropagation()}>
              {WB_MENU.filter((m) => !openTabs.includes(m.id)).map((m) => (
                <li key={m.id}>
                  <button
                    type="button"
                    onClick={() => {
                      onOpen(m.id);
                      setMenu(false);
                    }}
                  >
                    <span className="menu-icon" aria-hidden>
                      {m.icon}
                    </span>
                    <span className="menu-label">{WB_LABELS[m.id]}</span>
                    {m.key && <kbd>{m.key}</kbd>}
                  </button>
                </li>
              ))}
              {openTabs.length >= WB_MENU.length && <li className="empty">全部已打开</li>}
            </ul>
          )}
        </div>
        <span className="btab-spacer" />
        <button
          type="button"
          className="btab-panel-close"
          title="收起面板 ⌘J"
          onClick={onClosePanel}
        >
          ×
        </button>
      </div>
      <div className="wb-body">{children}</div>
      {menu && (
        <button
          type="button"
          className="menu-backdrop"
          aria-label="关闭菜单"
          onClick={() => setMenu(false)}
        />
      )}
    </aside>
  );
}
