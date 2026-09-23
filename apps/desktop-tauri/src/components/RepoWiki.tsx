import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { renderMarkdown } from "../lib/markdown";
import { readWorkspaceFile } from "./FileTree";

export type WikiItem = {
  path: string;
  title: string;
  bytes: number;
};

export type WikiList = {
  root: string;
  items: WikiItem[];
  excluded: number;
  truncated: boolean;
};

export async function listRepoWiki(root?: string): Promise<WikiList> {
  return invoke<WikiList>("list_repo_wiki", { root: root ?? null });
}

/** D12 Repo Wiki：左目录 + 右正文；敏感文档已整篇排除。 */
export function RepoWiki({ root }: { root?: string }) {
  const [list, setList] = useState<WikiList | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [content, setContent] = useState<string>("");
  const [err, setErr] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [q, setQ] = useState("");

  const refresh = async () => {
    setLoading(true);
    setErr(null);
    try {
      const r = await listRepoWiki(root);
      setList(r);
      if (!selected && r.items[0]) {
        setSelected(r.items[0].path);
        await open(r.items[0].path);
      }
    } catch (e) {
      setErr(String(e));
    } finally {
      setLoading(false);
    }
  };

  const open = async (path: string) => {
    setSelected(path);
    setContent("");
    try {
      const f = await readWorkspaceFile(root, path);
      setContent(f.content ?? (f.binary ? "二进制文件" : ""));
    } catch (e) {
      setContent(String(e));
    }
  };

  const filtered = useMemo(() => {
    const items = list?.items ?? [];
    const s = q.trim().toLowerCase();
    if (!s) return items;
    return items.filter(
      (i) =>
        i.title.toLowerCase().includes(s) || i.path.toLowerCase().includes(s),
    );
  }, [list, q]);

  if (!list) {
    return (
      <div className="tree-empty">
        <p className="muted">Repo Wiki · 根目录入口 + docs/*.md</p>
        <button type="button" className="primary" onClick={() => void refresh()} disabled={loading}>
          {loading ? "加载中…" : "加载 Wiki"}
        </button>
        {err && <p className="error-text">{err}</p>}
      </div>
    );
  }

  return (
    <div className="repo-wiki">
      <div className="tree-head">
        <input
          className="wiki-search"
          value={q}
          placeholder="搜索标题…"
          onChange={(e) => setQ(e.target.value)}
        />
        <button type="button" className="ghost-btn" onClick={() => void refresh()}>
          刷新
        </button>
      </div>
      {list.excluded > 0 && (
        <p className="muted warn-line">
          ⚠ {list.excluded} 篇未收录（疑似敏感信息，整篇排除）
        </p>
      )}
      <div className="wiki-layout">
        <nav className="wiki-nav">
          {filtered.map((i) => (
            <button
              key={i.path}
              type="button"
              className={selected === i.path ? "active" : ""}
              onClick={() => void open(i.path)}
              title={i.path}
            >
              {i.title}
            </button>
          ))}
          {filtered.length === 0 && <p className="muted">无匹配</p>}
        </nav>
        <article className="wiki-body">
          {content ? (
            <div className="wiki-md">{renderMarkdown(content)}</div>
          ) : (
            <p className="muted">选择左侧文档</p>
          )}
        </article>
      </div>
    </div>
  );
}
