import { useState } from "react";
import { Icon, type IconName } from "./Icon";

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

export const WB_ICON: Record<WorkbenchId, IconName> = {
  review: "review",
  terminal: "terminal",
  files: "files",
  browser: "browser",
  chat: "chat",
  sim: "sim",
  goal: "goal",
  subagents: "subagents",
  automations: "automations",
};

export const WB_MENU: { id: WorkbenchId; key: string }[] = [
  { id: "review", key: "⇧⌘G" },
  { id: "terminal", key: "⌘`" },
  { id: "browser", key: "⌘T" },
  { id: "files", key: "⌘P" },
  { id: "chat", key: "⌥⌘S" },
  { id: "sim", key: "" },
  { id: "goal", key: "" },
  { id: "subagents", key: "" },
  { id: "automations", key: "" },
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
              <Icon name={WB_ICON[id]} size={14} className="btab-ico" />
              {WB_LABELS[id]}
            </button>
            <button
              type="button"
              className="btab-close"
              title="关闭标签"
              aria-label={`关闭 ${WB_LABELS[id]}`}
              onClick={() => onCloseTab(id)}
            >
              <Icon name="close" size={12} />
            </button>
          </div>
        ))}
        <div className={`btab-add ${menu ? "open" : ""}`}>
          <button
            type="button"
            className={`btab-plus ${menu ? "active" : ""}`}
            title="打开标签页"
            aria-label="打开标签页"
            aria-expanded={menu}
            onClick={(e) => {
              e.stopPropagation();
              setMenu((v) => !v);
            }}
          >
            <Icon name="plus" size={14} />
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
                      <Icon name={WB_ICON[m.id]} size={15} />
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
          aria-label="收起面板"
          onClick={onClosePanel}
        >
          <Icon name="chevron-right" size={14} />
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
