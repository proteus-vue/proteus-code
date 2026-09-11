#!/usr/bin/env python3
"""
协议确定性校验 —— 同一 Op 序列必须产出同一 EventMsg 序列。

这是"多宿主共享内核"能成立的前提，也是 ADR-0003 的验证手段。
本脚本用 Python 参照实现 Rust 侧的状态机，对 golden 用例做回放比对。
（Rust 侧同一逻辑由 cargo test 覆盖；此处验证的是协议语义本身。）
"""
import json, os, sys

HERE = os.path.dirname(__file__)
GOLDEN = os.path.join(HERE, "..", "golden", "protocol_replay.json")

# ---- 状态机参照实现（对应 neo-core::Session::transition） ----
IDLE, PLANNING, EXECUTING, AWAITING = "Idle", "Planning", "Executing", "AwaitingApproval"
SEQ = [0]

def next_event_id(prefix="ev"):
    SEQ[0] += 1
    return f"{prefix}_{SEQ[0]:04d}"   # ID 规则必须确定：单调递增计数器

def transition(state, op):
    kind = op["op"]
    if kind == "shutdown":
        return IDLE
    if kind == "interrupt":
        return IDLE          # 中断在安全点生效
    if kind == "shell":
        return state          # 用户直输命令不影响会话状态机
    if state == IDLE:
        if kind in ("user_turn", "goal_set"):
            return AWAITING if op.get("needs_approval") else PLANNING
    if state == PLANNING:
        return EXECUTING
    if state == EXECUTING:
        return EXECUTING
    if state == AWAITING:
        if kind == "approve":
            return EXECUTING
        return AWAITING
    return state

def emits(state_before, op, state_after):
    """给定状态转移，产出确定的事件序列。

    关键：等待审批时不发 turn_complete —— 轮次尚未结束。
    """
    kind = op["op"]
    out = []
    if kind == "user_turn":
        # 用户消息先回显：转录里必须能看出"当时问的是什么"
        out.append({"kind": "user_submitted", "text": op.get("text", "")})
        out.append({"kind": "turn_started", "turn_id": next_event_id("t")})
        out.append({"kind": "agent_message_done", "text": op.get("echo", "")})
        if op.get("tool"):
            out.append({"kind": "tool_call_begin", "id": next_event_id("tc"), "name": op["tool"]})
            if op.get("needs_approval"):
                out.append({"kind": "approval_request", "id": next_event_id("ap")})
                return out          # 等审批，本轮不结束
            out.append({"kind": "tool_call_end", "id": next_event_id("tc"), "exit_code": 0})
        out.append({"kind": "turn_complete", "input_tokens": 100, "output_tokens": 20})
    elif kind == "approve":
        out.append({"kind": "tool_call_end", "id": next_event_id("tc"), "exit_code": 0})
        out.append({"kind": "turn_complete", "input_tokens": 10, "output_tokens": 5})
    elif kind == "interrupt":
        out.append({"kind": "turn_complete", "input_tokens": 0, "output_tokens": 0})
    elif kind == "shell":
        # 用户直输的 shell：不经模型，只产生一对工具事件
        out.append({"kind": "tool_call_begin", "id": next_event_id("tc"), "name": "bash"})
        out.append({"kind": "tool_call_end", "id": next_event_id("tc"), "exit_code": 0})
    elif kind == "goal_set":
        out.append({"kind": "turn_started", "turn_id": next_event_id("t")})
        out.append({"kind": "goal_progress", "done": 0, "total": op.get("total", 1)})
    elif kind == "goal_pause":
        out.append({"kind": "checkpoint_saved", "checkpoint_id": next_event_id("cp")})
    elif kind == "shutdown":
        out.append({"kind": "shutdown_complete"})
    return out

def run(ops):
    SEQ[0] = 0
    state, events = IDLE, []
    for op in ops:
        before = state
        state = transition(state, op)
        for e in emits(before, op, state):
            events.append(e)
    return events

def main():
    path = os.path.normpath(GOLDEN)
    if not os.path.exists(path):
        print(f"FAIL 找不到 golden: {path}")
        return 1
    cases = json.load(open(path, encoding="utf-8"))["cases"]

    print("=" * 62)
    print("协议确定性校验 · Op 序列 -> EventMsg 序列")
    print("=" * 62)
    fails = 0
    for c in cases:
        name = c["name"]
        ops = c["ops"]
        expected = c["expected_event_kinds"]

        r1 = run(ops)
        r2 = run(ops)   # 跑两遍：验证确定性（ID 计数器重置后结果一致）

        kinds1 = [e["kind"] for e in r1]
        kinds2 = [e["kind"] for e in r2]

        ok_deterministic = (json.dumps(r1, sort_keys=True) == json.dumps(r2, sort_keys=True))
        ok_expect = (kinds1 == expected)

        status = "PASS" if (ok_deterministic and ok_expect) else "FAIL"
        if status == "FAIL":
            fails += 1
        print(f"  [{status}] {name}")
        print(f"         events: {kinds1}")
        if not ok_expect:
            print(f"         expected: {expected}")
        if not ok_deterministic:
            print("         ⚠ 两次回放结果不一致 —— 违反确定性")

    print("-" * 62)
    if fails:
        print(f"❌ {fails}/{len(cases)} 个用例失败")
        return 1
    print(f"✅ {len(cases)}/{len(cases)} 个用例通过：协议序列确定且与期望一致")
    return 0

if __name__ == "__main__":
    sys.exit(main())
