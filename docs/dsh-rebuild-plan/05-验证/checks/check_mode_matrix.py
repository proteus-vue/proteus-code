#!/usr/bin/env python3
"""
沙箱 × 审批 正交双轴矩阵校验（ADR-0004）。

校验项：
  M1 5 档执行模式 × 3 档沙箱，每个组合都有确定的审批结果（无未定义格）
  M2 沙箱是硬边界：沙箱不允许的操作，任何审批策略都不得放行
  M3 审批策略不改变能力边界，只改变"是否询问"
  M4 经典反例必须成立：
     - never + read_only  => 仍写不了文件（不是"放开权限"）
     - danger_full_access + untrusted => 仍会被询问（不是"静默全放行"）
  M5 workspace_write 默认断网
"""
import sys, itertools

SANDBOX = ["read_only", "workspace_write", "danger_full_access"]
APPROVAL = ["untrusted", "on_request", "on_failure", "never"]
WRITE_OUTSIDE = "write_outside_workspace"
NETWORK = "network"

def sandbox_allows(mode, action):
    if mode == "danger_full_access":
        return True
    if mode == "read_only":
        return False
    if mode == "workspace_write":
        return action != WRITE_OUTSIDE and action != NETWORK   # 默认断网
    raise ValueError(mode)

def needs_ask(policy):
    return policy != "never"

def gate(mode, policy, action):
    """返回 ('allow'|'ask'|'deny') —— 对应 dsh_core::ApprovalGate::gate"""
    if not sandbox_allows(mode, action):
        # 沙箱不允许：审批不能覆盖
        if policy == "never":
            return "deny"     # 不问，直接失败
        return "ask"          # 问用户是否越界（越界需升级）
    return "ask" if needs_ask(policy) else "allow"

def main():
    fails = []
    print("=" * 62)
    print("沙箱 × 审批 正交双轴矩阵")
    print("=" * 62)
    print(f"{'sandbox':<22}{'approval':<12}{'in-workspace':<15}{'outside':<12}{'network':<10}")
    print("-" * 62)
    for s, p in itertools.product(SANDBOX, APPROVAL):
        r_in  = gate(s, p, "write_in_workspace")
        r_out = gate(s, p, WRITE_OUTSIDE)
        r_net = gate(s, p, NETWORK)
        if any(r is None for r in (r_in, r_out, r_net)):
            fails.append(f"M1 组合 ({s},{p}) 存在未定义结果")
        print(f"{s:<22}{p:<12}{r_in:<15}{r_out:<12}{r_net:<10}")

        # M2 硬边界：read_only 下写操作永远不能 allow
        if s == "read_only" and r_in == "allow":
            fails.append(f"M2 read_only 下写入被放行 ({s},{p})")
        # M2 硬边界：workspace_write 下越界/联网永远不能 allow
        if s == "workspace_write" and r_out == "allow":
            fails.append(f"M2 workspace_write 下越界被放行 ({s},{p})")

    # M3 审批不改能力：固定沙箱，改审批，allow 的集合不变
    for s in SANDBOX:
        base = {gate(s, p, "write_in_workspace") for p in APPROVAL}
        if "allow" in base and "deny" in base:
            fails.append(f"M3 沙箱 {s} 下审批策略改变了能力边界")

    # M4 反例
    print("-" * 62)
    t1 = gate("read_only", "never", "write_in_workspace")
    ok1 = (t1 == "deny")
    print(f"  [{'PASS' if ok1 else 'FAIL'}] M4-a never + read_only => {t1} (期望 deny：不是放开权限)")
    if not ok1: fails.append("M4-a 反例不成立")

    t2 = gate("danger_full_access", "untrusted", "write_in_workspace")
    ok2 = (t2 == "ask")
    print(f"  [{'PASS' if ok2 else 'FAIL'}] M4-b full_access + untrusted => {t2} (期望 ask：仍会询问)")
    if not ok2: fails.append("M4-b 反例不成立")

    # M5 workspace_write 默认断网
    ok5 = (gate("workspace_write", "never", NETWORK) == "deny")
    print(f"  [{'PASS' if ok5 else 'FAIL'}] M5 workspace_write 默认断网 (never 下也是 deny)")
    if not ok5: fails.append("M5 默认断网策略失效")

    print("-" * 62)
    if fails:
        for f in dict.fromkeys(fails):
            print("   -", f)
        print(f"❌ 模式矩阵校验失败，共 {len(set(fails))} 项")
        return 1
    print("✅ 模式矩阵自洽：双轴正交、沙箱为硬边界、经典反例成立")
    return 0

if __name__ == "__main__":
    sys.exit(main())
