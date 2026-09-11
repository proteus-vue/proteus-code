#!/usr/bin/env python3
"""
架构守卫 —— 强制依赖方向，防止内核被宿主反向污染。

校验项：
  A1 依赖只能向下，不得向上（如 L2 不得依赖 L3/L4/L5）
  A2 依赖图无环
  A3 三个宿主 crate 之间无相互依赖
  A4 neo-protocol 不得依赖任何业务 crate
  A5 workspace 声明的 member 必须实际存在，且 package name == 目录名

这是 ADR-0001（固定内核）与 ADR-0003（多宿主）的机器化保证。
"""
import os, re, sys, json
from collections import defaultdict

ROOT = os.environ.get("NEO_ROOT", os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", ".."))

LAYER = {
    # 编号 = 真实依赖深度：neo-protocol 零业务依赖，是整个依赖图的基石
    "neo-protocol":      0,  # L0 PROTOCOL（基石）
    "neo-sandbox":       1,  # L1 PLATFORM
    "neo-platform":      1,  # L1 PLATFORM
    "neo-core":          2,  # L2
    "neo-capability":    3,  # L3
    "neo-sandbox-local": 3,  # L3 PROVIDER（SandboxBackend 实现）
    "neo-llm-deepseek":  3,  # L3 PROVIDER（ModelProvider 实现）
    "neo-session-local": 3,  # L3 PROVIDER（SessionPersistence 实现）
    "neo-session-store": 3,  # L3 PROVIDER（多会话库：列举/新建/删除）
    "neo-skill-loader":  3,  # L3 PROVIDER（技能目录发现与加载）
    "neo-instructions":  3,  # L3 PROVIDER（AGENTS.md 级联加载）
    "neo-providers":     3,  # L3 PROVIDER（服务商注册表：用户级 JSON）
    "neo-orchestration": 4,  # L4
    "neo-host-tui":      5,  # L5
    "neo-host-desktop":  5,  # L5
    "neo-host-web":      5,  # L5
    "neo-exec":          5,  # L5
    "neo-cli":           5,  # L5
}
SIDE = {"neo-session", "neo-config"}   # 旁挂，任何层可用
HOSTS = {"neo-host-tui", "neo-host-desktop", "neo-host-web", "neo-exec"}
LAYER_NAME = {0: "L0 protocol", 1: "L1 platform", 2: "L2 core",
              3: "L3 capability", 4: "L4 orchestration", 5: "L5 host"}

def parse_cargo(path):
    """极简 TOML 解析：够用于本仓库自写的 Cargo.toml。"""
    name = None
    deps = []
    section = None
    for raw in open(path, encoding="utf-8"):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("["):
            section = line.strip("[]").strip()
            continue
        m = re.match(r'^([A-Za-z0-9_\-]+)\s*=\s*(.+)$', line)
        if not m:
            continue
        k, v = m.group(1), m.group(2)
        if section == "package" and k == "name":
            name = v.strip().strip('"')
        elif section == "dependencies" and k != "workspace":
            # name = { path = "../x" }  或  name = "1"
            pm = re.search(r'path\s*=\s*"[^"]*/([A-Za-z0-9_\-]+)"', v)
            if pm:
                deps.append(pm.group(1))
    return name, deps

def main():
    failures, checks = [], []

    ws_path = os.path.join(ROOT, "Cargo.toml")
    if not os.path.exists(ws_path):
        print(f"FAIL 找不到 workspace: {ws_path}")
        return 1
    members = re.findall(r'"(crates/[^"]+)"', open(ws_path, encoding="utf-8").read())

    # A5
    for m in members:
        d = os.path.join(ROOT, m)
        if not os.path.isdir(d):
            failures.append(f"A5 workspace member 不存在: {m}")
            continue
        n, _ = parse_cargo(os.path.join(d, "Cargo.toml"))
        if n != os.path.basename(m):
            failures.append(f"A5 package name '{n}' != 目录名 '{os.path.basename(m)}'")
    checks.append(("A5", "workspace member 与 package name 一致", not any(f.startswith("A5") for f in failures)))

    graph = {}
    for m in members:
        n, deps = parse_cargo(os.path.join(ROOT, m, "Cargo.toml"))
        if n:
            graph[n] = deps

    # A1 依赖方向
    for crate, deps in sorted(graph.items()):
        if crate not in LAYER:
            continue
        my = LAYER[crate]
        for d in deps:
            if d in SIDE:
                continue
            dl = LAYER.get(d)
            if dl is None:
                failures.append(f"A1 {crate} 依赖了未知 crate: {d}")
            elif dl > my:
                failures.append(
                    f"A1 向上依赖：{crate}({LAYER_NAME[my]}) -> {d}({LAYER_NAME[dl]})")
    checks.append(("A1", "依赖只能向下，无向上依赖", not any(f.startswith("A1") for f in failures)))

    # A2 环检测（白路径 DFS）
    color = defaultdict(int)
    cyc = []
    def dfs(n, stack):
        color[n] = 1
        for d in graph.get(n, []):
            if color[d] == 1:
                cyc.append(" -> ".join(stack + [d]))
            elif color[d] == 0:
                dfs(d, stack + [d])
        color[n] = 2
    for n in list(graph):
        if color[n] == 0:
            dfs(n, [n])
    for c in set(cyc):
        failures.append(f"A2 依赖成环: {c}")
    checks.append(("A2", "依赖图无环", not cyc))

    # A3 宿主之间不互相依赖
    for h in sorted(HOSTS):
        for d in graph.get(h, []):
            if d in HOSTS and d != h:
                failures.append(f"A3 宿主间依赖：{h} -> {d}")
    checks.append(("A3", "宿主之间无相互依赖", not any(f.startswith("A3") for f in failures)))

    # A4 protocol 零业务依赖
    pdeps = [d for d in graph.get("neo-protocol", []) if d not in SIDE and d in LAYER]
    for d in pdeps:
        failures.append(f"A4 neo-protocol 依赖业务 crate: {d}")
    checks.append(("A4", "neo-protocol 无业务依赖", not pdeps))

    print("=" * 62)
    print("架构守卫 · 依赖方向校验")
    print("=" * 62)
    for n in sorted(graph, key=lambda x: (LAYER.get(x, 9), x)):
        tag = LAYER_NAME.get(LAYER.get(n, 9), "side" if n in SIDE else "?")
        dep_str = ", ".join(sorted(graph[n])) or "-"
        print(f"  [{tag:16}] {n:20} -> {dep_str}")
    print("-" * 62)
    for code, desc, ok in checks:
        print(f"  [{'PASS' if ok else 'FAIL'}] {code}  {desc}")
    print("-" * 62)
    if failures:
        print(f"❌ 架构守卫失败，共 {len(failures)} 项：")
        for f in failures:
            print("   -", f)
        return 1
    print("✅ 架构守卫全部通过：依赖方向合法、无环、宿主隔离、协议层纯净")
    return 0

if __name__ == "__main__":
    sys.exit(main())
