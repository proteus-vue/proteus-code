import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export type FileTreeResult = {
  root: string;
  entries: string[];
  truncated: boolean;
};

/** 与协议 format_file_ref 同语义：空格路径加引号 → @ref */
export function formatFileRef(path: string): string {
  const p = path.replace(/\/$/, "");
  if (!p || p.includes("\n") || p.includes("\r")) return "";
  const need = /\s/.test(p) || p.startsWith('"') || p.startsWith("'");
  if (!need) return `@${p}`;
  if (!p.includes('"')) return `@"${p}"`;
  if (!p.includes("'")) return `@'${p}'`;
  return `@"${p.replace(/"/g, '\\"')}"`;
}

export async function listWorkspace(root?: string): Promise<FileTreeResult> {
  return invoke<FileTreeResult>("list_workspace", { root: root ?? null });
}

export type FilePreview = {
  path: string;
  binary: boolean;
  content: string | null;
  bytes: number;
  truncated?: boolean;
};

export async function readWorkspaceFile(
  root: string | undefined,
  path: string,
): Promise<FilePreview> {
  return invoke<FilePreview>("read_workspace_file", {
    root: root ?? null,
    path,
  });
}

type Node = {
  name: string;
  path: string;
  dir: boolean;
  children?: Node[];
};

function buildTree(entries: string[]): Node[] {
  const root: Node[] = [];
  const byPath = new Map<string, Node>();
  const ensureDir = (path: string, name: string): Node => {
    const existing = byPath.get(path);
    if (existing) return existing;
    const node: Node = { name, path, dir: true, children: [] };
    byPath.set(path, node);
    const parent = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
    if (parent) {
      const p = ensureDir(parent, parent.includes("/") ? parent.slice(parent.lastIndexOf("/") + 1) : parent);
      p.children!.push(node);
    } else {
      root.push(node);
    }
    return node;
  };

  for (const raw of entries) {
    const isDir = raw.endsWith("/");
    const path = isDir ? raw.slice(0, -1) : raw;
    if (!path) continue;
    const name = path.includes("/") ? path.slice(path.lastIndexOf("/") + 1) : path;
    if (isDir) {
      ensureDir(path, name);
    } else {
      const parent = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
      const node: Node = { name, path, dir: false };
      if (parent) ensureDir(parent, parent.includes("/") ? parent.slice(parent.lastIndexOf("/") + 1) : parent).children!.push(node);
      else root.push(node);
    }
  }

  const sortRec = (nodes: Node[]) => {
    nodes.sort((a, b) => {
      if (a.dir !== b.dir) return a.dir ? -1 : 1;
      return a.name.localeCompare(b.name);
    });
    nodes.forEach((n) => n.children && sortRec(n.children));
  };
  sortRec(root);
  return root;
}

export function FileTree({
  onSelect,
  root,
  selected,
}: {
  root?: string;
  selected?: string | null;
  onSelect: (relPath: string) => void;
}) {
  const [raw, setRaw] = useState<string[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [truncated, setTruncated] = useState(false);
  const [open, setOpen] = useState<Set<string>>(() => new Set());
  const [loading, setLoading] = useState(false);

  const refresh = async () => {
    setLoading(true);
    setErr(null);
    try {
      const r = await listWorkspace(root);
      setRaw(r.entries);
      setTruncated(r.truncated);
      const tops = r.entries.filter((e) => e.endsWith("/")).map((e) => e.slice(0, -1));
      setOpen(new Set(tops));
    } catch (e) {
      setErr(String(e));
    } finally {
      setLoading(false);
    }
  };

  const tree = useMemo(() => (raw ? buildTree(raw) : []), [raw]);

  const toggle = (path: string) => {
    setOpen((s) => {
      const n = new Set(s);
      if (n.has(path)) n.delete(path);
      else n.add(path);
      return n;
    });
  };

  const render = (nodes: Node[], depth: number): React.ReactNode =>
    nodes.map((n) => {
      const key = n.path;
      if (n.dir) {
        const isOpen = open.has(key);
        return (
          <div key={key}>
            <button
              type="button"
              className="tree-row"
              style={{ paddingLeft: 8 + depth * 12 }}
              onClick={() => toggle(key)}
            >
              <span className="tree-twist">{isOpen ? "▾" : "▸"}</span>
              <span className="tree-dir">{n.name}</span>
            </button>
            {isOpen && n.children && render(n.children, depth + 1)}
          </div>
        );
      }
      const active = selected === n.path;
      return (
        <button
          key={key}
          type="button"
          className={`tree-row ${active ? "active" : ""}`}
          style={{ paddingLeft: 8 + depth * 12 }}
          onClick={() => onSelect(n.path)}
          title="预览文件"
        >
          <span className="tree-twist" />
          <span className="tree-file">{n.name}</span>
        </button>
      );
    });

  return (
    <div className="file-tree">
      <div className="tree-head">
        <button type="button" className="ghost-btn" onClick={() => void refresh()} disabled={loading}>
          {loading ? "扫描中…" : "刷新"}
        </button>
        <button
          type="button"
          className="ghost-btn"
          onClick={() =>
            setOpen(
              new Set(
                raw?.filter((e) => e.endsWith("/")).map((e) => e.slice(0, -1)) ?? [],
              ),
            )
          }
        >
          全部折叠
        </button>
      </div>
      {err && <p className="muted error-text">{err}</p>}
      {!raw && !err && (
        <div className="tree-empty">
          <p className="muted">加载工作区文件</p>
          <button type="button" className="primary" onClick={() => void refresh()}>
            扫描
          </button>
        </div>
      )}
      {raw && (
        <>
          <div className="tree-body">{render(tree, 0)}</div>
          {truncated && <p className="muted tree-trunc">已截断（条目上限）</p>}
        </>
      )}
    </div>
  );
}
