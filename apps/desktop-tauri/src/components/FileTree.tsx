import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { fsGetMetadata, fsReadDirectory, fsReadFile } from "../lib/rpc";
import { Icon } from "./Icon";

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

export type FilePreview = {
  path: string;
  binary: boolean;
  content: string | null;
  bytes: number;
  truncated?: boolean;
};

const FS_SKIP = new Set([
  ".git",
  "node_modules",
  "target",
  "dist",
  "build",
  ".venv",
  "__pycache__",
  ".next",
  ".cache",
  ".neo",
  ".DS_Store",
]);

/** 协议 fs/readDirectory 递归（深度/条目有界）；失败退回 Tauri walk。 */
async function walkFs(absRoot: string, dir: string, depth: number, out: string[]): Promise<void> {
  if (depth > 8 || out.length >= 800) return;
  const r = await fsReadDirectory(dir);
  const names = [...(r.entries ?? [])].sort((a, b) => a.localeCompare(b));
  for (const name of names) {
    if (FS_SKIP.has(name) || out.length >= 800) continue;
    const child = dir === "/" ? `/${name}` : `${dir}/${name}`;
    const rel = child.startsWith(absRoot)
      ? child.slice(absRoot.length).replace(/^\//, "")
      : child.replace(/^\//, "");
    let isDir = false;
    try {
      const md = await fsGetMetadata(child);
      isDir = Boolean(md.is_dir);
    } catch {
      isDir = false;
    }
    if (isDir) {
      out.push(rel + "/");
      await walkFs(absRoot, child, depth + 1, out);
    } else {
      out.push(rel);
    }
  }
}

export async function listWorkspace(root?: string): Promise<FileTreeResult> {
  try {
    const base = (root ?? "").trim();
    if (!base) throw new Error("no root");
    const abs = base.startsWith("/") ? base : "/" + base;
    const out: string[] = [];
    await walkFs(abs, abs, 0, out);
    return { root: abs, entries: out, truncated: out.length >= 800 };
  } catch {
    return invoke<FileTreeResult>("list_workspace", { root: root ?? null });
  }
}

export async function readWorkspaceFile(
  root: string | undefined,
  path: string,
): Promise<FilePreview> {
  try {
    const abs = path.startsWith("/")
      ? path
      : ((root ?? "").replace(/\/$/, "") + "/" + path).replace(/\/{2,}/g, "/");
    const r = await fsReadFile(abs);
    const content = r.content ?? "";
    let binary = false;
    for (let i = 0; i < Math.min(content.length, 8000); i++) {
      if (content.charCodeAt(i) === 0) {
        binary = true;
        break;
      }
    }
    return {
      path,
      binary,
      content: binary ? null : content,
      bytes: r.bytes ?? content.length,
      truncated: r.truncated,
    };
  } catch {
    return invoke<FilePreview>("read_workspace_file", {
      root: root ?? null,
      path,
    });
  }
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
  refreshKey = 0,
}: {
  root?: string;
  selected?: string | null;
  onSelect: (relPath: string) => void;
  /** 变化即重新扫描（fs/changed / files_changed 驱动） */
  refreshKey?: number;
}) {
  const [raw, setRaw] = useState<string[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [truncated, setTruncated] = useState(false);
  const [open, setOpen] = useState<Set<string>>(() => new Set());
  const [loading, setLoading] = useState(false);
  const [openSnap, setOpenSnap] = useState<Set<string> | null>(null);

  const refresh = async (keepOpen = false) => {
    setLoading(true);
    setErr(null);
    try {
      const r = await listWorkspace(root);
      setRaw(r.entries);
      setTruncated(r.truncated);
      const tops = r.entries.filter((e) => e.endsWith("/")).map((e) => e.slice(0, -1));
      if (keepOpen) {
        // 保留用户已展开的目录，只补新的顶层
        setOpen((prev) => {
          const n = new Set(prev);
          for (const t of tops) if (prev.size === 0 || prev.has(t) || openSnap?.has(t)) n.add(t);
          // 首次仍全开
          if (prev.size === 0) return new Set(tops);
          return n;
        });
      } else {
        setOpen(new Set(tops));
        setOpenSnap(new Set(tops));
      }
    } catch (e) {
      setErr(String(e));
    } finally {
      setLoading(false);
    }
  };

  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;

  useEffect(() => {
    if (refreshKey > 0) void refreshRef.current(true);
  }, [refreshKey]);

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
              <span className="tree-twist">
                <Icon name={isOpen ? "chevron-down" : "chevron-right"} size={12} />
              </span>
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
        <button type="button" className="ghost-btn" onClick={() => void refresh(false)} disabled={loading}>
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
          <button type="button" className="primary" onClick={() => void refresh(false)}>
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
