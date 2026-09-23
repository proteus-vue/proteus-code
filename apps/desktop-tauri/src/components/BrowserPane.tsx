/**
 * 浏览器工作台（RpBrowser · IA-30/32/33）：
 * 地址栏 + 预览 + **点选元素入对话** + Wiki。
 *
 * 点选不依赖页内 CSP 脚本：父页面对**同源 iframe**（srcdoc/抓取页）
 * 直接挂 mousemove/click，画蓝框 + 元素信息浮层（对齐 ZCode 观感）。
 */
import { useCallback, useEffect, useRef, useState } from "react";
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
  size?: string;
  color?: string;
  font?: string;
};

type Mode = "web" | "wiki";

function normalizeUrl(raw: string): string {
  const s = raw.trim();
  if (!s) return "";
  if (/^https?:\/\//i.test(s) || s.startsWith("file://") || s.startsWith("/")) return s;
  if (s.includes(".") && !s.includes(" ")) return `https://${s}`;
  return `https://www.google.com/search?q=${encodeURIComponent(s)}`;
}

function cssPath(el: Element): string {
  if (el.id) return `#${CSS.escape(el.id)}`;
  const parts: string[] = [];
  let n: Element | null = el;
  let i = 0;
  while (n && n.nodeType === 1 && n !== n.ownerDocument?.body && i < 6) {
    const tag = n.tagName.toLowerCase();
    let idx = 1;
    let s = n.previousElementSibling;
    while (s) {
      if (s.tagName === n.tagName) idx++;
      s = s.previousElementSibling;
    }
    parts.unshift(`${tag}:nth-child(${idx})`);
    n = n.parentElement;
    i++;
  }
  return `body > ${parts.join(" > ")}`;
}

const HL_STYLE_ID = "neo-pick-style";
const BANNER_ID = "neo-pick-banner";

function ensurePickChrome(doc: Document) {
  if (!doc.getElementById(HL_STYLE_ID)) {
    const st = doc.createElement("style");
    st.id = HL_STYLE_ID;
    st.textContent = `
      .neo-hl { outline: 2px solid #3b82f6 !important; outline-offset: 2px !important; }
      #${BANNER_ID} {
        position: fixed; left: 12px; bottom: 48px; z-index: 2147483647;
        background: #1f2937; color: #f9fafb; font: 12px/1.45 ui-monospace, monospace;
        padding: 8px 10px; border-radius: 10px; box-shadow: 0 8px 24px rgba(0,0,0,.35);
        max-width: min(420px, 90vw); pointer-events: none; white-space: pre-wrap;
      }
    `;
    (doc.head || doc.documentElement).appendChild(st);
  }
}

function clearHover(doc: Document) {
  doc.querySelectorAll(".neo-hl").forEach((e) => e.classList.remove("neo-hl"));
}

function removeChrome(doc: Document) {
  clearHover(doc);
  doc.getElementById(BANNER_ID)?.remove();
}

function describeEl(el: Element): { w: number; h: number; color: string; font: string } {
  const cs = (el.ownerDocument?.defaultView as Window | null)?.getComputedStyle(el);
  const r = el.getBoundingClientRect();
  return {
    w: Math.round(r.width),
    h: Math.round(r.height),
    color: cs?.color || "",
    font: (cs?.font || "").slice(0, 48),
  };
}

/** 父页对同源 iframe 挂点选；返回 detach。失败返回 null（跨域）。 */
function attachPicker(
  iframe: HTMLIFrameElement,
  pageUrl: string,
  onPick: (el: WebElementPick) => void,
): (() => void) | null {
  let doc: Document;
  try {
    doc = iframe.contentDocument as Document;
  } catch {
    return null;
  }
  if (!doc || !doc.body) return null;

  ensurePickChrome(doc);

  const onMove = (ev: MouseEvent) => {
    const t = ev.target as Element | null;
    if (!t || t.nodeType !== 1) return;
    clearHover(doc);
    t.classList.add("neo-hl");
    const meta = describeEl(t);
    let banner = doc.getElementById(BANNER_ID);
    if (!banner) {
      banner = doc.createElement("div");
      banner.id = BANNER_ID;
      doc.body.appendChild(banner);
    }
    const tag = t.tagName.toLowerCase();
    banner.textContent = [
      tag,
      `${meta.w}×${meta.h}`,
      meta.color ? `Color ${meta.color}` : "",
      meta.font ? `Font ${meta.font}` : "",
      "点击加入对话 · Esc 取消",
    ]
      .filter(Boolean)
      .join("\n");
    // 靠近光标
    banner.style.left = `${Math.min(ev.clientX + 14, (doc.defaultView?.innerWidth ?? 800) - 200)}px`;
    banner.style.top = `${Math.min(ev.clientY + 14, (doc.defaultView?.innerHeight ?? 600) - 80)}px`;
    banner.style.bottom = "auto";
  };

  const onClick = (ev: MouseEvent) => {
    ev.preventDefault();
    ev.stopPropagation();
    const t = ev.target as Element | null;
    if (!t || t.nodeType !== 1) return;
    const meta = describeEl(t);
    onPick({
      url: pageUrl || doc.URL || "",
      selector: cssPath(t),
      tag: t.tagName.toLowerCase(),
      text: (t.textContent || "").trim().slice(0, 4000),
      html: t.outerHTML.slice(0, 8000),
      size: `${meta.w}×${meta.h}`,
      color: meta.color,
      font: meta.font,
    });
  };

  const onKey = (ev: KeyboardEvent) => {
    if (ev.key === "Escape") detach();
  };

  const detach = () => {
    doc.removeEventListener("mousemove", onMove, true);
    doc.removeEventListener("click", onClick, true);
    (doc.defaultView || window).removeEventListener("keydown", onKey, true);
    removeChrome(doc);
  };

  doc.addEventListener("mousemove", onMove, true);
  doc.addEventListener("click", onClick, true);
  (doc.defaultView || window).addEventListener("keydown", onKey, true);
  return detach;
}

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
  const detachRef = useRef<(() => void) | null>(null);
  const iframeRef = useRef<HTMLIFrameElement>(null);

  const teardownPick = useCallback(() => {
    detachRef.current?.();
    detachRef.current = null;
  }, []);

  useEffect(() => {
    if (mode !== "web") return;
    if (
      filePreview &&
      /\.html?$/i.test(filePreview.path) &&
      !filePreview.binary &&
      filePreview.content
    ) {
      setSrcDoc(filePreview.content);
      setUrl(`file://${filePreview.path}`);
      setInput(`file://${filePreview.path}`);
      setErr(null);
      setPickMode(false);
    }
  }, [filePreview, mode]);

  useEffect(() => () => teardownPick(), [teardownPick]);

  // 收旧 postMessage（兼容）
  useEffect(() => {
    const onMsg = (e: MessageEvent) => {
      const d = e.data as { type?: string } | null;
      if (!d || d.type !== "neo-pick-element") return;
      setPickMode(false);
      teardownPick();
      onPickElement?.(e.data as unknown as WebElementPick);
    };
    window.addEventListener("message", onMsg);
    return () => window.removeEventListener("message", onMsg);
  }, [onPickElement, teardownPick]);

  const attachFromIframe = useCallback(
    (iframe: HTMLIFrameElement | null) => {
      teardownPick();
      if (!pickMode || !iframe) return;
      const det = attachPicker(iframe, url, (el) => {
        setPickMode(false);
        teardownPick();
        onPickElement?.(el);
      });
      if (!det) {
        setErr("无法读取该页面（跨域）— 点「选取」会先抓取为可注入预览");
      } else {
        detachRef.current = det;
        setErr(null);
      }
    },
    [onPickElement, pickMode, teardownPick, url],
  );

  // srcdoc / key 变化后挂上
  useEffect(() => {
    if (!pickMode || mode !== "web") return;
    const id = window.setTimeout(() => attachFromIframe(iframeRef.current), 50);
    return () => window.clearTimeout(id);
  }, [attachFromIframe, iframeKey, pickMode, mode, srcDoc]);

  const navigate = (raw: string) => {
    const next = normalizeUrl(raw);
    if (!next) return;
    teardownPick();
    setPickMode(false);
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

  const reload = () => {
    teardownPick();
    setIframeKey((k) => k + 1);
  };

  const external = async () => {
    try {
      await openUrl(url);
    } catch (e) {
      setErr(String(e));
    }
  };

  const startPick = useCallback(async () => {
    setMode("web");
    setErr(null);
    // 远程且尚无可注入 HTML → 先抓
    const needFetch =
      /^https?:\/\//i.test(url) &&
      (!srcDoc || !srcDoc.includes("<html") && !srcDoc.includes("<!DOCTYPE"));
    // 更稳：远程始终抓一次，保证同源可操纵
    const isHttp = /^https?:\/\//i.test(url);
    if (isHttp) {
      try {
        const html = await fetchUrl(url);
        if (html && /</.test(html)) {
          setSrcDoc(html);
        } else {
          setErr("抓取页面失败，无法点选");
          return;
        }
      } catch (e) {
        setErr(String(e));
        return;
      }
    } else if (!srcDoc && !/file:/i.test(url)) {
      setErr("请先打开网页，或在文件中选择 .html");
      return;
    }
    void needFetch;
    setPickMode(true);
    setIframeKey((k) => k + 1);
  }, [srcDoc, url]);

  const stopPick = () => {
    teardownPick();
    setPickMode(false);
    setIframeKey((k) => k + 1);
  };

  // 本地 html 用 srcdoc；远程抓取后也用 srcdoc，保证可 attach
  const useSrc = Boolean(srcDoc);

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
          选取中：悬停 <b style={{ color: "#3b82f6" }}>蓝框</b> + 元素信息 · 点击加入对话 · Esc 结束
          <button type="button" className="ghost-btn" onClick={stopPick}>
            结束
          </button>
        </div>
      )}

      {mode === "web" ? (
        <div className="br-frame">
          {useSrc ? (
            <iframe
              key={`doc-${iframeKey}`}
              ref={iframeRef}
              title="页面预览"
              className="br-iframe"
              // srcdoc 同源：父页可直接操作 DOM（不塞页内脚本，避开 CSP）
              sandbox="allow-scripts allow-same-origin allow-modals"
              srcDoc={srcDoc ?? ""}
              onLoad={(e) => {
                // 等 body 就绪再 attach
                requestAnimationFrame(() => attachFromIframe(e.currentTarget));
              }}
            />
          ) : (
            <iframe
              key={`web-${iframeKey}`}
              ref={iframeRef}
              title="网页预览"
              className="br-iframe"
              src={url}
              sandbox="allow-scripts allow-forms allow-same-origin"
              onLoad={() => {
                if (pickMode) attachFromIframe(iframeRef.current);
                else setErr(null);
              }}
              onError={() => setErr("页面加载失败 — 可点右侧「系统打开」")}
            />
          )}
          <p className="br-hint">
            {pickMode
              ? "蓝框悬停 · 点击元素写入对话；远程页点「选取」会先抓取为可注入预览。"
              : "部分站点禁止内嵌；失败用「系统打开」。选取可把元素送入对话。"}
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
