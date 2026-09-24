/**
 * xterm + 内核 PTY（IA-9 / compose desktop-xterm-pty）。
 * 拉模型：onData → execWrite；空闲 execWrite("") 排水；fit → resize。
 */
import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import {
  commandExecSession,
  execResize,
  execTerminate,
  execWrite,
} from "../lib/rpc";

type PtySnap = {
  session_id?: string;
  output?: string;
  running?: boolean;
  exit_code?: number | null;
  mode?: string;
};

function writeOut(term: Terminal, s: string | undefined | null) {
  if (s) term.write(s);
}

export function TerminalPane({ shell = "bash" }: { shell?: string }) {
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const sessionIdRef = useRef<string | null>(null);
  const pollRef = useRef<number | null>(null);
  const aliveRef = useRef(true);
  const drainingRef = useRef(false);
  /** 排水/写占用时排队按键，禁止直接丢（审查 critical） */
  const pendingInputRef = useRef("");
  const flushRef = useRef<(() => Promise<void>) | null>(null);

  useEffect(() => {
    aliveRef.current = true;
    const host = hostRef.current;
    if (!host) return;

    const term = new Terminal({
      convertEol: true,
      cursorBlink: true,
      fontSize: 13,
      fontFamily:
        "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
      scrollback: 5000,
      theme: {
        background: "#1e1e1e",
        foreground: "#d4d4d4",
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    try {
      fit.fit();
    } catch {
      /* 容器尚无尺寸时忽略 */
    }
    termRef.current = term;
    fitRef.current = fit;

    term.writeln("\x1b[90m启动 PTY 会话…\x1b[0m");

    let disposed = false;
    pendingInputRef.current = "";

    const stopPoll = () => {
      if (pollRef.current != null) {
        window.clearInterval(pollRef.current);
        pollRef.current = null;
      }
    };

    const terminateQuiet = async (id: string | null) => {
      if (id) await execTerminate(id).catch(() => undefined);
    };

    const applySnap = (snap: PtySnap | unknown) => {
      const s = (snap ?? {}) as PtySnap;
      writeOut(term, s.output);
      if (s.running === false) {
        stopPoll();
        const code = s.exit_code;
        term.writeln(
          `\r\n\x1b[90m会话结束${code != null ? ` (exit ${code})` : ""}\x1b[0m`,
        );
        sessionIdRef.current = null;
        pendingInputRef.current = "";
      }
    };

    /** 串行写：占用时把 data 追加到 pending，结束后冲刷 */
    const writeSerial = async (data: string) => {
      if (drainingRef.current) {
        pendingInputRef.current += data;
        return;
      }
      const id = sessionIdRef.current;
      if (!id) return;
      drainingRef.current = true;
      let payload = data;
      try {
        // 合并排队中的输入，一次 write 发出
        if (pendingInputRef.current) {
          payload += pendingInputRef.current;
          pendingInputRef.current = "";
        }
        const r = await execWrite(id, payload);
        if (aliveRef.current) applySnap(r);
      } catch (e) {
        if (aliveRef.current) {
          term.writeln(`\r\n\x1b[31m${String(e)}\x1b[0m`);
          // 出错先 terminate，避免 PTY 孤儿（审查 critical）
          const sid = sessionIdRef.current;
          sessionIdRef.current = null;
          pendingInputRef.current = "";
          stopPoll();
          await terminateQuiet(sid);
        }
      } finally {
        drainingRef.current = false;
        // 冲刷排队按键
        if (
          aliveRef.current &&
          sessionIdRef.current &&
          pendingInputRef.current
        ) {
          const next = pendingInputRef.current;
          pendingInputRef.current = "";
          void writeSerial(next);
        }
      }
    };
    flushRef.current = () => writeSerial("");

    const start = async () => {
      try {
        const snap = (await commandExecSession(shell, 120, 32)) as PtySnap;
        if (disposed || !aliveRef.current) {
          if (snap.session_id) {
            await execTerminate(snap.session_id).catch(() => undefined);
          }
          return;
        }
        sessionIdRef.current = snap.session_id ?? null;
        term.clear();
        writeOut(term, snap.output);
        if (snap.mode) {
          term.writeln(`\x1b[90m${snap.mode} · ${snap.session_id ?? ""}\x1b[0m`);
        }
        if (snap.running === false) {
          term.writeln("\x1b[90m进程立即退出\x1b[0m");
          return;
        }
        try {
          fit.fit();
          if (sessionIdRef.current) {
            const dims = fit.proposeDimensions?.();
            if (dims && dims.cols && dims.rows) {
              await execResize(sessionIdRef.current, dims.cols, dims.rows);
            }
          }
        } catch {
          /* ignore */
        }
        // 空闲排水：write("") 仍 mem::take 增量输出
        stopPoll();
        pollRef.current = window.setInterval(() => {
          if (!aliveRef.current || drainingRef.current) return;
          const id = sessionIdRef.current;
          if (!id) return;
          void writeSerial("");
        }, 180);
      } catch (e) {
        if (!disposed) {
          term.writeln(`\x1b[31m启动失败：${String(e)}\x1b[0m`);
        }
      }
    };

    const dataSub = term.onData((data) => {
      const id = sessionIdRef.current;
      if (!id) return;
      // 排水占用 → 入队，不丢键（审查 critical）
      if (drainingRef.current) {
        pendingInputRef.current += data;
        return;
      }
      void writeSerial(data);
    });

    const onWinResize = () => {
      try {
        fit.fit();
      } catch {
        /* ignore */
      }
      const id = sessionIdRef.current;
      if (!id) return;
      const dims = fit.proposeDimensions?.();
      if (dims && dims.cols && dims.rows) {
        void execResize(id, dims.cols, dims.rows).catch(() => undefined);
      }
    };
    window.addEventListener("resize", onWinResize);
    const ro =
      typeof ResizeObserver !== "undefined"
        ? new ResizeObserver(() => onWinResize())
        : null;
    ro?.observe(host);

    void start();

    return () => {
      disposed = true;
      aliveRef.current = false;
      stopPoll();
      dataSub.dispose();
      window.removeEventListener("resize", onWinResize);
      ro?.disconnect();
      const id = sessionIdRef.current;
      sessionIdRef.current = null;
      pendingInputRef.current = "";
      void terminateQuiet(id);
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
      flushRef.current = null;
    };
  }, [shell]);

  return (
    <div className="wb-section terminal-pane">
      <div className="term-note">
        xterm · PTY 会话 · 走沙箱 · 不经模型 · 关闭标签即 terminate
      </div>
      <div className="xterm-host" ref={hostRef} />
    </div>
  );
}
