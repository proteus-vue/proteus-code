/**
 * 浏览器工作台（RpBrowser · IA-30）：地址栏 + 预览 + Wiki。
 * 本地 HTML 用 srcdoc；http(s) 用 iframe（失败可「系统打开」）。
 */
import { useEffect, useState } from "react";
import { Icon } from "./Icon";
import { openUrl } from "../lib/rpc";
import { RepoWiki } from "./RepoWiki";
import type { FilePreview } from "./FileTree";

type Mode = "web" | "wiki";

function normalizeUrl(raw: string): string {
  const s = raw.trim();
  if (!s) return "";
  if (/^https?:\/\//i.test(s) || s.startsWith("file://") || s.startsWith("/")) return s;
  if (s.includes(".") && !s.includes(" ")) return `https://${s}`;
  return `https://www.google.com/search?q=${encodeURIComponent(s)}`;
}

export function BrowserPane({
  filePreview,
  workspaceRoot,
}: {
  filePreview: FilePreview | null;
  workspaceRoot?: string;
}) {
  const [mode, setMode] = useState<Mode>("web");
  const [url, setUrl] = useState(
    () => localStorage.getItem("neo-browser-url") || "https://example.com",
  );
  const [input, setInput] = useState(url);
  const [history, setHistory] = useState<string[]>([url]);
  const [hi, setHi] = useState(0);
  const [srcDoc, setSrcDoc] = useState<string | null>(null);
  const [iframeKey, setIframeKey] = useState(0);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    if (mode !== "web") return;
    if (filePreview && /\.html?$/i.test(filePreview.path) && !filePreview.binary && filePreview.content) {
      setSrcDoc(filePreview.content);
      setUrl(`file://${filePreview.path}`);
      setInput(`file://${filePreview.path}`);
      setErr(null);
    }
  }, [filePreview, mode]);

  const navigate = (raw: string) => {
    const next = normalizeUrl(raw);
    if (!next) return;
    setUrl(next);
    setInput(next);
    setSrcDoc(null);
    setErr(null);
    localStorage.setItem("neo-browser-url", next);
    setHistory((h) => {
      const cut = h.slice(0, hi + 1);
      if (cut[cut.length - 1] === next) return cut;
      return [...cut, next];
    });
    setHi((i) => i + 1);
    setIframeKey((k) => k + 1);
  };

  const goBack = () => {
    if (hi <= 0) return;
    const n = hi - 1;
    setHi(n);
    setUrl(history[n]);
    setInput(history[n]);
    setSrcDoc(null);
    setIframeKey((k) => k + 1);
  };

  const goFwd = () => {
    if (hi >= history.length - 1) return;
    const n = hi + 1;
    setHi(n);
    setUrl(history[n]);
    setInput(history[n]);
    setSrcDoc(null);
    setIframeKey((k) => k + 1);
  };

  const reload = () => setIframeKey((k) => k + 1);

  const external = async () => {
    try {
      await openUrl(url);
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <div className="wb-section browser-pane">
      <div className="br-toolbar">
        <div className="br-nav">
          <button type="button" className="br-btn" disabled={hi <= 0} onClick={goBack} title="后退" aria-label="后退">
            <Icon name="chevron-left" size={15} />
          </button>
          <button
            type="button"
            className="br-btn"
            disabled={hi >= history.length - 1}
            onClick={goFwd}
            title="前进"
            aria-label="前进"
          >
            <Icon name="chevron-right" size={15} />
          </button>
          <button type="button" className="br-btn" onClick={reload} title="刷新" aria-label="刷新">
            <Icon name="search" size={14} />
          </button>
        </div>
        <form
          className="br-url"
          onSubmit={(e) => {
            e.preventDefault();
            navigate(input);
          }}
        >
          <input
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder="输入网址或搜索，Enter 打开"
            spellCheck={false}
            autoComplete="off"
            aria-label="地址"
          />
        </form>
        <button type="button" className="br-btn" onClick={() => void external()} title="系统浏览器打开" aria-label="系统打开">
          <Icon name="browser" size={15} />
        </button>
        <div className="br-seg" role="tablist">
          <button
            type="button"
            role="tab"
            aria-selected={mode === "web"}
            className={mode === "web" ? "active" : ""}
            onClick={() => setMode("web")}
          >
            网页
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={mode === "wiki"}
            className={mode === "wiki" ? "active" : ""}
            onClick={() => setMode("wiki")}
          >
            Wiki
          </button>
        </div>
      </div>

      {err && <div className="br-err">{err}</div>}

      {mode === "web" ? (
        <div className="br-frame">
          {srcDoc ? (
            <iframe
              key={`doc-${iframeKey}`}
              title="本地页面"
              className="br-iframe"
              sandbox="allow-scripts allow-same-origin"
              srcDoc={srcDoc}
            />
          ) : (
            <iframe
              key={`web-${iframeKey}`}
              title="网页预览"
              className="br-iframe"
              src={url}
              sandbox="allow-scripts allow-forms allow-same-origin"
              onLoad={() => setErr(null)}
              onError={() => setErr("页面加载失败 — 可点右侧「系统打开」")}
            />
          )}
          <p className="br-hint">
            部分站点禁止内嵌；失败请用 <b>系统打开</b>。在「文件」里选 .html 可本地预览。
            {filePreview && /\.html?$/i.test(filePreview.path) && (
              <> · 当前：<code>{filePreview.path}</code></>
            )}
          </p>
        </div>
      ) : (
        <div className="br-wiki">
          <RepoWiki root={workspaceRoot} />
        </div>
      )}
    </div>
  );
}
