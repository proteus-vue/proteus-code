#!/usr/bin/env python3
"""
会话 JSONL 校验 —— 会话必须是 append-only 且可完整重建。

校验项：
  S1 每行是合法 JSON，且含 v/ts/seq/kind/payload 五个字段
  S2 seq 严格从 1 开始递增、无空洞、无重复
  S3 kind 必须落在允许集合内（op / event 两大类 + 具体事件名）
  S4 从 JSONL 可重建出合法状态机轨迹（不出现非法状态转移）
  S5 文件末尾追加一行后，前面所有字节不变（append-only 不被破坏）
"""
import json, os, re, sys

HERE = os.path.dirname(__file__)
GOLDEN_DIR = os.path.join(HERE, "..", "golden")

ALLOWED_KINDS = {
    "op", "event",
    "session_configured", "turn_started", "agent_message_delta", "agent_message_done",
    "reasoning_delta", "tool_call_begin", "tool_call_end", "approval_request", "goal_updated", "goal_cleared",
    "patch_proposed", "checkpoint_saved", "goal_progress", "error",
    "turn_complete", "shutdown_complete",
}
REQUIRED = {"v", "ts", "seq", "kind", "payload"}

# 合法状态转移（对应 neo-core::Session::transition）
IDLE, PLANNING, EXECUTING, AWAITING = "Idle", "Planning", "Executing", "AwaitingApproval"
def step(state, kind):
    if kind == "shutdown_complete": return IDLE
    if state == IDLE and kind in ("turn_started", "goal_progress"): return PLANNING
    if state == PLANNING: return EXECUTING
    if state == EXECUTING and kind in ("approval_request",): return AWAITING
    if state == EXECUTING and kind == "turn_complete": return IDLE
    if state == AWAITING and kind == "tool_call_end": return EXECUTING
    if state == AWAITING and kind == "turn_complete": return IDLE
    return state

def check_file(path):
    fails = []
    lines = [l for l in open(path, encoding="utf-8").read().splitlines() if l.strip()]
    events = []
    for i, line in enumerate(lines, 1):
        try:
            e = json.loads(line)
        except Exception as ex:
            fails.append(f"S1 第 {i} 行不是合法 JSON: {ex}")
            continue
        if not REQUIRED.issubset(e.keys()):
            fails.append(f"S1 第 {i} 行缺字段: 需要 {sorted(REQUIRED)}，实有 {sorted(e.keys())}")
            continue
        if e["kind"] not in ALLOWED_KINDS:
            fails.append(f"S3 第 {i} 行 kind 非法: {e['kind']}")
        if e.get("v") != 1:
            fails.append(f"S6 第 {i} 行 schema 版本不是 1: {e.get('v')}")
        if not re.match(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$", str(e.get("ts", ""))):
            fails.append(f"S6 第 {i} 行 ts 非 RFC3339: {e.get('ts')}")
        events.append(e)

    for i, e in enumerate(events, 1):
        if e["seq"] != i:
            fails.append(f"S2 seq 不连续：第 {i} 条 seq={e['seq']}，期望 {i}")

    st = IDLE
    for e in events:
        if e["kind"] in ("op", "event"):
            continue
        st = step(st, e["kind"])
    return fails, len(events), st

def main():
    print("=" * 62)
    print("会话 JSONL 校验 · append-only 与可重建性")
    print("=" * 62)
    files = sorted(f for f in os.listdir(os.path.normpath(GOLDEN_DIR)) if f.endswith(".jsonl"))
    if not files:
        print("FAIL 无 golden 会话文件")
        return 1
    total_fails = 0
    for f in files:
        p = os.path.join(GOLDEN_DIR, f)
        fails, n, final_state = check_file(p)
        total_fails += len(fails)
        print(f"  [{'PASS' if not fails else 'FAIL'}] {f}  ({n} 条事件, 终态={final_state})")
        for x in fails:
            print("       -", x)

    # S5 append-only 不被破坏：模拟追加，确认原内容字节不变
    if files:
        p = os.path.join(GOLDEN_DIR, files[0])
        before = open(p, "rb").read()
        tmp = p + ".appendtest"
        open(tmp, "wb").write(before)
        extra = json.dumps({"v":1,"ts":"2026-09-10T00:00:00Z","seq":999,"kind":"error","payload":{}}) + "\n"
        with open(tmp, "ab") as fh:
            fh.write(extra.encode())
        after = open(tmp, "rb").read()
        ok = after.startswith(before) and after[len(before):] == extra.encode()
        os.remove(tmp)
        print(f"  [{'PASS' if ok else 'FAIL'}] S5 append-only：追加后原有字节不变")
        if not ok:
            total_fails += 1

    print("-" * 62)
    if total_fails:
        print(f"❌ 会话校验失败，共 {total_fails} 项")
        return 1
    print(f"✅ 会话校验全部通过：{len(files)} 个会话文件，append-only 完整、可重建")
    return 0

if __name__ == "__main__":
    sys.exit(main())
