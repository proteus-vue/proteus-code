import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ExecMode } from "../lib/rpc";
import {
  configMcpServerReload,
  configRead,
  configRequirementsRead,
  configValueWrite,
  externalAgentDetect,
  externalAgentImport,
  feedbackUpload,
  hooksList,
  listSkills,
  marketplaceAdd,
  marketplaceRemove,
  marketplaceUpgrade,
  mcpServerStatusList,
  permissionProfileList,
  pluginInstall,
  pluginList,
  pluginReconcile,
  pluginUninstall,
  skillsConfigWrite,
  skillsExtraRootsSet,
  threadAttachmentList,
  type MigrationItem,
} from "../lib/rpc";
import type { PluginInfo, SkillInfo } from "../lib/protocol";
import { Icon } from "./Icon";
import { Select } from "./Select";

const EXEC_MODES: { id: ExecMode; label: string }[] = [
  { id: "plan", label: "Plan" },
  { id: "confirm_before", label: "确认" },
  { id: "default", label: "Default" },
  { id: "auto_edit", label: "自动编辑" },
  { id: "full_access", label: "完全访问" },
];

/** ⌘, 设置页（PRODUCT-IA §5 / IA-13）—— 模态，不改三区结构。 */
export function SettingsModal({
  open,
  onClose,
  theme,
  onTheme,
  model,
  models,
  onModel,
  execMode,
  onMode,
  protocolVersion,
  methodCount,
  workspaceRoot,
}: {
  open: boolean;
  onClose: () => void;
  theme: "light" | "dark";
  onTheme: (t: "light" | "dark") => void;
  model: string;
  models: { name: string; description?: string }[];
  onModel: (name: string) => void;
  execMode: ExecMode;
  onMode: (m: ExecMode) => void;
  protocolVersion: number | string;
  methodCount: number;
  workspaceRoot: string;
}) {
  const closeRef = useRef<HTMLButtonElement>(null);
  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [skillsErr, setSkillsErr] = useState<string | null>(null);
  const [plugins, setPlugins] = useState<PluginInfo[]>([]);
  const [markets, setMarkets] = useState<{ name: string; source: string }[]>([]);
  const [pluginErr, setPluginErr] = useState<string | null>(null);
  const [mktName, setMktName] = useState("");
  const [mktSource, setMktSource] = useState("");
  const [busyPlugin, setBusyPlugin] = useState<string | null>(null);
  const [configSummary, setConfigSummary] = useState<string>("");
  const [hooks, setHooks] = useState<string>("—");
  const [mcpLine, setMcpLine] = useState<string>("—");
  const [permLine, setPermLine] = useState<string>("—");
  const [reqLine, setReqLine] = useState<string>("—");
  const [diagNote, setDiagNote] = useState<string>("");
  const [extraRoot, setExtraRoot] = useState("");
  const [migrations, setMigrations] = useState<MigrationItem[]>([]);
  const [migNote, setMigNote] = useState("");
  const [attachments, setAttachments] = useState<string>("—");

  const reloadPlugins = useCallback(async () => {
    try {
      const r = await pluginList();
      setPlugins(r.plugins ?? []);
      setMarkets(r.marketplaces ?? []);
      setPluginErr(null);
    } catch (e) {
      setPluginErr(String(e));
    }
  }, []);

  useEffect(() => {
    if (!open) return;
    requestAnimationFrame(() => closeRef.current?.focus());
    void listSkills()
      .then((r) => {
        setSkills(r.skills ?? []);
        setSkillsErr(null);
      })
      .catch((e) => setSkillsErr(String(e)));
    void reloadPlugins();
    void configRead()
      .then((c) => {
        const bits = [
          c.model ? `model=${c.model}` : null,
          c.exec_mode ? `mode=${c.exec_mode}` : null,
          c.sandbox_mode ? `sandbox=${c.sandbox_mode}` : null,
        ].filter(Boolean);
        setConfigSummary(bits.join(" · ") || "—");
      })
      .catch(() => setConfigSummary("—"));
    void hooksList()
      .then((r) => setHooks(`${(r.hooks ?? []).length} 个 hooks`))
      .catch((e) => setHooks(String(e)));
    void mcpServerStatusList()
      .then((r) => {
        const s = r.servers ?? [];
        setMcpLine(
          s.length
            ? s.map((x) => `${x.name}:${x.status ?? "?"}`).join(" · ")
            : "无 MCP 服务器",
        );
      })
      .catch((e) => setMcpLine(String(e)));
    void permissionProfileList()
      .then((r) => {
        const cur = (r.profiles ?? []).find((p) => p.current);
        setPermLine(cur?.label || cur?.id || "—");
      })
      .catch((e) => setPermLine(String(e)));
    void configRequirementsRead()
      .then((r) => {
        const n = (r.requirements ?? []).length;
        setReqLine(r.configFile ? `${n} 项 · ${r.configFile}` : `${n} 项`);
      })
      .catch((e) => setReqLine(String(e)));
    void threadAttachmentList()
      .then((r) =>
        setAttachments(
          (r.attachments ?? []).length
            ? `${(r.attachments ?? []).length} 条附件`
            : "无附件",
        ),
      )
      .catch((e) => setAttachments(String(e)));
    void externalAgentDetect({ includeHome: false })
      .then((r) => setMigrations(r.migrationItems ?? []))
      .catch(() => setMigrations([]));
  }, [open, reloadPlugins]);

  const shortcuts = useMemo(
    () =>
      [
        ["⌘N", "新建会话"],
        ["⌘K / ⌘P", "命令面板"],
        ["⌘B", "折叠/展开侧栏"],
        ["⌘J", "折叠/展开右栏"],
        ["⌘L", "聚焦输入"],
        ["⌘,", "设置"],
        ["⇧Tab", "循环执行模式"],
        ["Esc", "中断生成"],
        ["Enter（生成中）", "转向当前轮 turn/steer"],
      ] as const,
    [],
  );

  if (!open) return null;

  return (
    <div
      className="palette-backdrop settings-backdrop"
      onClick={onClose}
      role="presentation"
    >
      <div
        className="settings-modal"
        role="dialog"
        aria-label="设置"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="settings-head">
          <h2>设置</h2>
          <button
            ref={closeRef}
            type="button"
            className="icon-btn"
            onClick={onClose}
            title="关闭 Esc / ⌘,"
            aria-label="关闭设置"
          >
            <Icon name="close" size={16} />
          </button>
        </header>

        <div className="settings-body">
          <section className="settings-section">
            <h3>外观</h3>
            <div className="settings-row">
              <span>主题</span>
              <div className="seg">
                <button
                  type="button"
                  className={theme === "light" ? "active" : ""}
                  onClick={() => onTheme("light")}
                >
                  浅色
                </button>
                <button
                  type="button"
                  className={theme === "dark" ? "active" : ""}
                  onClick={() => onTheme("dark")}
                >
                  深色
                </button>
              </div>
            </div>
          </section>

          <section className="settings-section">
            <h3>会话</h3>
            <div className="settings-row">
              <span>模型</span>
              <Select
                value={model}
                ariaLabel="模型"
                options={(models.length ? models : [{ name: model }]).map((m) => ({
                  value: m.name,
                  label: m.name,
                  hint: m.description,
                }))}
                onChange={onModel}
              />
            </div>
            <div className="settings-row">
              <span>执行模式</span>
              <Select
                value={execMode}
                ariaLabel="执行模式"
                options={EXEC_MODES.map((m) => ({ value: m.id, label: m.label }))}
                onChange={(v) => onMode(v as ExecMode)}
              />
            </div>
            <div className="settings-row">
              <span>工作区</span>
              <code className="settings-path" title={workspaceRoot}>
                {workspaceRoot || "—"}
              </code>
            </div>
            <div className="settings-row">
              <span>config/read</span>
              <code className="settings-path" title={configSummary}>
                {configSummary}
              </code>
            </div>
            <div className="settings-row">
              <span>configRequirements</span>
              <code className="settings-path" title={reqLine}>
                {reqLine}
              </code>
            </div>
            <div className="settings-row">
              <span>hooks</span>
              <code className="settings-path">{hooks}</code>
            </div>
            <div className="settings-row">
              <span>MCP</span>
              <code className="settings-path" title={mcpLine}>
                {mcpLine}
              </code>
              <button
                type="button"
                className="ghost-btn"
                onClick={() => {
                  void configMcpServerReload()
                    .then((r) =>
                      setDiagNote(
                        `mcp 重读：${(r.servers ?? []).join(",") || "无"}${r.note ? ` · ${r.note}` : ""}`,
                      ),
                    )
                    .catch((e) => setDiagNote(String(e)));
                }}
              >
                重读
              </button>
            </div>
            <div className="settings-row">
              <span>附件</span>
              <code className="settings-path">{attachments}</code>
            </div>
            <div className="settings-row">
              <span>权限档</span>
              <code className="settings-path">{permLine}</code>
            </div>
            <div className="settings-row">
              <span>写配置</span>
              <div className="mkt-add">
                <input
                  placeholder="keyPath 如 model"
                  onKeyDown={(e) => {
                    if (e.key !== "Enter") return;
                    const key = (e.target as HTMLInputElement).value.trim();
                    if (!key) return;
                    void configValueWrite(key, model, "replace")
                      .then(() => setDiagNote(`已写 config ${key}=${model}`))
                      .catch((err) => setDiagNote(String(err)));
                    (e.target as HTMLInputElement).value = "";
                  }}
                />
                <span className="muted">Enter 写入当前 model</span>
              </div>
            </div>
            {diagNote && <p className="muted">{diagNote}</p>}
          </section>

          <section className="settings-section">
            <h3>技能</h3>
            {skillsErr && <p className="muted error-text">{skillsErr}</p>}
            {!skillsErr && skills.length === 0 && (
              <p className="muted">未加载到技能（skills/list 空表）</p>
            )}
            <ul className="settings-list">
              {skills.map((s) => (
                <li key={s.name} title={s.description}>
                  <code>${s.name}</code>
                  <span className="muted">{s.description || ""}</span>
                  <button
                    type="button"
                    className="ghost-btn"
                    onClick={() => {
                      // 默认禁用；再点启用（无选选择器时启用=清空该名禁用）
                      void skillsConfigWrite(false, s.name)
                        .then(() => setDiagNote(`已禁用 $${s.name}（重启会话生效）`))
                        .catch((e) => setDiagNote(String(e)));
                    }}
                  >
                    禁用
                  </button>
                </li>
              ))}
            </ul>
            <div className="settings-row">
              <span>额外技能根</span>
              <div className="mkt-add">
                <input
                  value={extraRoot}
                  placeholder="/abs/path/to/skills"
                  onChange={(e) => setExtraRoot(e.target.value)}
                />
                <button
                  type="button"
                  className="primary"
                  disabled={!extraRoot.trim()}
                  onClick={() => {
                    void skillsExtraRootsSet([extraRoot.trim()])
                      .then(() => {
                        setDiagNote("extraRoots 已写入（重启 app-server 生效）");
                        setExtraRoot("");
                      })
                      .catch((e) => setDiagNote(String(e)));
                  }}
                >
                  设置
                </button>
              </div>
            </div>
          </section>

          <section className="settings-section">
            <h3>插件市场</h3>
            <p className="muted">
              本地磁盘市场 · 无账号 · 只复制资源不执行 · plugin/share 不做
            </p>
            <div className="settings-row">
              <span>市场源</span>
              <div className="mkt-add">
                <input
                  value={mktName}
                  placeholder="名称 local"
                  onChange={(e) => setMktName(e.target.value)}
                />
                <input
                  value={mktSource}
                  placeholder="/abs/path/to/market"
                  onChange={(e) => setMktSource(e.target.value)}
                />
                <button
                  type="button"
                  className="primary"
                  disabled={!mktName.trim() || !mktSource.trim()}
                  onClick={() => {
                    void marketplaceAdd(mktName.trim(), mktSource.trim())
                      .then(() => {
                        setMktName("");
                        setMktSource("");
                        return reloadPlugins();
                      })
                      .catch((e) => setPluginErr(String(e)));
                  }}
                >
                  注册
                </button>
                <button
                  type="button"
                  className="ghost-btn"
                  onClick={() => {
                    void marketplaceUpgrade()
                      .then(() => reloadPlugins())
                      .then(() => setDiagNote("marketplace/upgrade 完成"))
                      .catch((e) => setPluginErr(String(e)));
                  }}
                >
                  刷新市场
                </button>
                <button
                  type="button"
                  className="ghost-btn"
                  onClick={() => {
                    void pluginReconcile()
                      .then((r) =>
                        setDiagNote(
                          `reconcile：存活 ${r.alive ?? 0} · 剔除 ${(r.removed ?? []).join(",") || "无"}`,
                        ),
                      )
                      .catch((e) => setPluginErr(String(e)));
                  }}
                >
                  对账
                </button>
              </div>
            </div>
            {markets.length > 0 && (
              <ul className="settings-list">
                {markets.map((m) => (
                  <li key={m.name}>
                    <code>{m.name}</code>
                    <span className="muted" title={m.source}>
                      {m.source}
                    </span>
                    <button
                      type="button"
                      className="ghost-btn"
                      onClick={() => {
                        void marketplaceRemove(m.name)
                          .then(() => reloadPlugins())
                          .catch((e) => setPluginErr(String(e)));
                      }}
                    >
                      移除
                    </button>
                  </li>
                ))}
              </ul>
            )}
            {pluginErr && <p className="muted error-text">{pluginErr}</p>}
            <ul className="settings-list">
              {plugins.map((p) => {
                const id = p.id || p.name || "?";
                return (
                  <li key={id}>
                    <Icon name="package" size={14} />
                    <code>{p.name || p.id}</code>
                    <span className="muted">{p.version || ""}</span>
                    <span className={`mkt-badge ${p.installed ? "on" : ""}`}>
                      {p.installed ? "已装" : "可装"}
                    </span>
                    <button
                      type="button"
                      className="ghost-btn"
                      disabled={busyPlugin === id}
                      onClick={() => {
                        setBusyPlugin(id);
                        const call = p.installed
                          ? pluginUninstall(String(p.id ?? p.name))
                          : pluginInstall(String(p.name ?? p.id));
                        void call
                          .then(() => reloadPlugins())
                          .catch((e) => setPluginErr(String(e)))
                          .finally(() => setBusyPlugin(null));
                      }}
                    >
                      {busyPlugin === id
                        ? "…"
                        : p.installed
                          ? "卸载"
                          : "安装"}
                    </button>
                  </li>
                );
              })}
              {plugins.length === 0 && !pluginErr && (
                <li className="muted">暂无可安装插件 · 先注册市场源</li>
              )}
            </ul>
          </section>

          <section className="settings-section">
            <h3>迁移 · 外部 Agent 配置</h3>
            <p className="muted">
              仅扫本地 CLAUDE.md / skills / mcp.json · externalAgentConfig/*
            </p>
            {migrations.length === 0 ? (
              <p className="muted">当前工作区未发现可迁移资产</p>
            ) : (
              <ul className="settings-list">
                {migrations.slice(0, 20).map((m, i) => (
                  <li key={`${m.itemType}-${i}`}>
                    <code>{m.itemType}</code>
                    <span className="muted" title={m.description}>
                      {m.description}
                    </span>
                  </li>
                ))}
              </ul>
            )}
            <div className="settings-row">
              <span>导入</span>
              <button
                type="button"
                className="primary"
                disabled={migrations.length === 0}
                onClick={() => {
                  void externalAgentImport(migrations)
                    .then((r) =>
                      setMigNote(
                        `导入 ${r.imported ?? 0} · 跳过 ${r.skipped ?? 0} · 失败 ${r.failed ?? 0}`,
                      ),
                    )
                    .catch((e) => setMigNote(String(e)));
                }}
              >
                全部导入
              </button>
              {migNote && <span className="muted">{migNote}</span>}
            </div>
          </section>

          <section className="settings-section">
            <h3>快捷键</h3>
            <ul className="settings-keys">
              {shortcuts.map(([k, label]) => (
                <li key={k}>
                  <kbd>{k}</kbd>
                  <span>{label}</span>
                </li>
              ))}
            </ul>
          </section>

          <section className="settings-section">
            <h3>关于</h3>
            <p className="muted">
              NEO Desktop · 本地 app-server · 协议 v{protocolVersion} ·{" "}
              {methodCount} methods · 不上传代码
            </p>
            <div className="settings-row">
              <span>反馈</span>
              <button
                type="button"
                className="ghost-btn"
                onClick={() => {
                  void feedbackUpload("user_note", "settings 关于区")
                    .then((r) =>
                      setDiagNote(r.localPath ? `本地收据 ${r.localPath}` : "已记录"),
                    )
                    .catch((e) => setDiagNote(String(e)));
                }}
              >
                写本地收据（feedback/upload）
              </button>
            </div>
          </section>
        </div>

        <footer className="settings-foot">
          <span className="muted">设置仅存本机（主题 localStorage；模型/模式走 session/configure）</span>
          <button type="button" className="primary" onClick={onClose}>
            完成
          </button>
        </footer>
      </div>
    </div>
  );
}
