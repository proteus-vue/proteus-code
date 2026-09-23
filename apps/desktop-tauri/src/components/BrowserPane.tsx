/**
 * 浏览器工作台（RpBrowser · IA-30/32）：
 * 地址栏 + 预览 + **点选元素加入对话** + Wiki。
 *
 * 点选：本地/抓取后的 srcdoc 注入脚本；跨域 iframe 先 fetch_url 转 srcdoc。
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Icon } from "./Icon";
import { fetchUrl, openUrl } from "../lib/rpc";
import { RepoWiki } from "./RepoWiki";
import type { FilePreview } from "./FileTree";

export type WebElementPick = {
  url: string;
  selector: string;
  tag: string;
  text: string;
  html: string;
};

type Mode = "web" | "wiki";

function normalizeUrl(raw: string): string {
  const s = raw.trim();
  if (!s) return "";
  if (/^https?:\/\//i.test(s) || s.startsWith("file://") || s.startsWith("/")) return s;
  if (s.includes(".") && !s.includes(" ")) return `https://${s}`;
  return `https://www.google.com/search?q=${encodeURIComponent(s)}`;
}

/** 注入 iframe 的点选脚本（仅 srcdoc/同源可控 DOM） */
const PICKER_SCRIPT = `
<style id="neo-pick-style">
  [data-neo-hl]{outline:2px solid #ff6a2b !important;outline-offset:2px !important;cursor:crosshair !important;}
  #neo-pick-banner{position:fixed;left:12px;bottom:12px;z-index:2147483647;background:#111;color:#fff;
    font:12px/1.4 system-ui,sans-serif;padding:6px 10px;border-radius:8px;pointer-events:none}
</style>
<script id="neo-pick-script">
(function(){
  if (window.__neoPick) return;
  var onMove = function(e){
    var t = e.target;
    if (!(t instanceof Element)) return;
    Array.prototype.forEach.call(document.querySelectorAll('[data-neo-hl]'), function(el){ el.removeAttribute('data-neo-hl'); });
    t.setAttribute('data-neo-hl','1');
    var b = document.getElementById('neo-pick-banner');
    if (!b){ b = document.createElement('div'); b.id='neo-pick-banner'; document.body.appendChild(b); }
    b.textContent = '选取元素 · 点击加入对话 · Esc 取消  ·  ' + t.tagName.toLowerCase();
  };
  var onClick = function(e){
    e.preventDefault(); e.stopPropagation();
    var t = e.target;
    if (!(t instanceof Element)) return;
    function sel(el){
      if (el.id) return '#' + CSS.escape(el.id);
      var parts = [];
      var n = el;
      while (n && n.nodeType === 1 && n !== document.body && parts.length < 5){
        var tag = n.tagName.toLowerCase();
        var i = 1, s = n.previousElementSibling;
        while (s){ if (s.tagName === n.tagName) i++; s = s.previousElementSibling; }
        parts.unshift(tag + ':nth-child(' + i + ')');
        n = n.parentElement;
      }
      return 'body > ' + parts.join(' > ');
    }
    var payload = {
      type: 'neo-pick-element',
      url: location.href,
      selector: sel(t),
      tag: t.tagName.toLowerCase(),
      text: (t.innerText || t.textContent || '').trim().slice(0, 4000),
      html: t.outerHTML.slice(0, 8000)
    };
    window.parent.postMessage(payload, '*');
    if (window.top) window.top.postMessage(payload, '*');
    cleanup();
  };
  var onKey = function(e){ if (e.key === 'Escape') cleanup(); };
  function cleanup(){
    document.removeEventListener('mousemove', onMove, true);
    document.removeEventListener('click', onClick, true);
    document.removeEventListener('keydown', onKey, true);
    Array.prototype.forEach.call(document.querySelectorAll('[data-neo-hl]'), function(el){ el.removeAttribute('data-neo-hl'); });
    var b = document.getElementById('neo-pick-banner'); if (b) b.remove();
    window.__neoPick = false;
  }
  window.__neoPick = true;
  document.addEventListener('mousemove', onMove, true);
  document.addEventListener('click', onClick, true);
  document.addEventListener('keydown', onKey, true);
  var b = document.getElementById('neo-pick-banner');
  if (!b){ b = document.createElement('div'); b.id='neo-pick-banner'; document.body.appendChild(b); }
  b.textContent = '选取元素 · 点击加入对话 · Esc 取消';
})();
`;

export function BrowserPane({
  filePreview,
  workspaceRoot,
  onPickElement,
}: {
  filePreview: FilePreview | null;
  workspaceRoot?: string;
  onPickElement?: (el: WebElementPick) => void;
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
  const [pickMode, setPickMode] = useState(false);
  const [picking, setPicking] = useState(false);
  const pickBusy = useRef(false);

  useEffect(() => {
    if (mode !== "web") return;
    if (filePreview && /\.html?$/i.test(filePreview.path) && !filePreview.binary && filePreview.content) {
      setSrcDoc(filePreview.content);
      setUrl(`file://${filePreview.path}`);
      setInput(`file://${filePreview.path}`);
      setErr(null);
    }
  }, [filePreview, mode]);

  // 父窗口收 iframe 点选结果
  useEffect(() => {
    const onMsg = (e: MessageEvent) => {
      const d = e.data;
      if (!d || typeof d !== "object" || (d as { type?: string }).type !== "neo-pick-element") return;
      const pick = d as unknown as WebElementPick;
      setPickMode(false);
      setPicking(false);
      onPickElement?.(pick);
    };
    window.addEventListener("message", onMsg);
    return () => window.removeEventListener("message", onMsg);
  }, [onPickElement]);

  // pickMode 时把脚本附到 srcdoc 并强制重载 iframe
  const viewDoc = useMemo(() => {
    if (!srcDoc) return null;
    if (!pickMode) return srcDoc;
    if (srcDoc.includes("neo-pick-script")) {
      // 已含脚本则不再追加；切换 pick 仍靠 iframeKey 重挂
      return srcDoc;
    }
    return `${srcDoc}\n${PICKER_SCRIPT}`;
  }, [srcDoc, pickMode]);

  useEffect(() => {
    if (mode === "web") setIframeKey((k) => k + 1);
  }, [pickMode, mode]);

  const navigate = (raw: string) => {
    const next = normalizeUrl(raw);
    if (!next) return;
    setUrl(next);
    setInput(next);
    setSrcDoc(null);
    setErr(null);
    setPickMode(false);
    setPicking(false);
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
    setPickMode(false);
    setIframeKey((k) => k + 1);
  };

  const goFwd = () => {
    if (hi >= history.length - 1) return;
    const n = hi + 1;
    setHi(n);
    setUrl(history[n]);
    setInput(history[n]);
    setSrcDoc(null);
    setPickMode(false);
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

  const startPick = useCallback(async () => {
    if (pickBusy.current) return;
    pickBusy.current = true;
    setPicking(true);
    setErr(null);
    try {
      // 远程页没有 srcdoc → 先抓 HTML，再注入点选脚本
      if (!srcDoc && /^https?:\/\//i.test(url)) {
        const html = await fetchUrl(url);
        if (!html || !html.includes("<")) {
          throw new Error("页面内容不像 HTML，无法点选");
        }
        setSrcDoc(html);
      } else if (!srcDoc && !/^https?:/i.test(url)) {
        throw new Error("当前无可注入页面 — 先打开网页或选择 .html 文件");
      }
      setPickMode(true);
      setMode("web");
      setIframeKey((k) => k + 1);
    } catch (e) {
      setPicking(false);
      setErr(String(e));
      setPickMode(false);
    } finally {
      pickBusy.current = false;
    }
  }, [srcDoc, url]);

  const stopPick = () => {
    setPickMode(false);
    setPicking(false);
    setIframeKey((k) => k + 1);
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
        <button
          type="button"
          className={`br-btn br-pick ${pickMode ? "active" : ""}`}
          onClick={() => {
            if (pickMode) stopPick();
            else void startPick();
          }}
          title={pickMode ? "取消选取 (Esc)" : "选取页面元素加入对话"}
          aria-label="选取元素"
          aria-pressed={pickMode}
        >
          <Icon name="edit" size={14} />
        </button>
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
      {pickMode && (
        <div className="br-pick-bar">
          选取模式：悬停高亮 · <b>点击元素</b>加入对话 · <kbd>Esc</kbd> 取消
          {picking && srcDoc ? " · 准备中…" : ""}
          <button type="button" className="ghost-btn" onClick={stopPick}>
            结束
          </button>
        </div>
      )}

      {mode === "web" ? (
        <div className="br-frame">
          {viewDoc ? (
            <iframe
              key={`doc-${iframeKey}`}
              title="页面预览"
              className="br-iframe"
              sandbox="allow-scripts allow-same-origin allow-modals"
              srcDoc={viewDoc}
              onLoad={() => {
                // 同源/ srcdoc 时也可补注入（fetch 后已含脚本则 noop）
                try {
                  const doc = (document.getElementById("neo-br") as HTMLIFrameElement | null)
                    ?.contentDocument;
                  void doc;
                } catch {
                  /* cross-origin */
                }
              }}
            />
          ) : (
            <iframe
              key={`web-${iframeKey}`}
              id="neo-br"
              title="网页预览"
              className="br-iframe"
              src={url}
              sandbox="allow-scripts allow-forms allow-same-origin"
              onLoad={() => setErr(null)}
              onError={() => setErr("页面加载失败 — 可点右侧「系统打开」")}
            />
          )}
          <p className="br-hint">
            {pickMode
              ? "点选模式开启中；远程页已尽量转为可注入预览。"
              : "部分站点禁止内嵌；失败请用「系统打开」。「选取」可把元素送入对话。"}
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
