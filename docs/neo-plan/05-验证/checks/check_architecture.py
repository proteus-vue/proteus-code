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
    # L1 BASE：宿主中立的文本语义（语义色调 / 显示宽度 / Markdown / 词法高亮）。
    # 它不是"平台抽象"，但依赖深度与平台同级（只依赖协议层与极小的纯 Rust
    # 解析库），故共用 1。放这里而不是 SIDE，是为了让它的依赖**也受 A1 检查**
    # —— SIDE 会跳过方向校验。
    # 必须沉到宿主之下的原因：TUI 与桌面 GUI 要用同一份 Tone/Markdown，
    # 而 A3 禁止宿主之间互相依赖。
    "neo-text":          1,  # L1 BASE（语义色调 / 宽度 / Markdown / 高亮）
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
    "neo-mcp":           3,  # L3 PROVIDER（MCP 客户端：外部工具服务器 → Tool 实现）
    "neo-agent-loader":  3,  # L3 PROVIDER（子代理定义发现与加载：Markdown → AgentSpec）
    # ── UI 栈（与内核轴正交的另一条轴）────────────────────────────────
    #
    # 编号含义仍是"真实依赖深度"，只是这条轴服务的是界面而不是内核。
    # 它们**不得**依赖内核轴（由 check_ui_layering.py 的 U1 强制），
    # 因此放在低层是准确的。
    "neo-ui-kit":        1,  # L1 UI 门面（唯一 pin GPUI 的地方）
    "neo-ui-render":     2,  # L2 UI 渲染缝
    "neo-ui-behavior":   3,  # L3 UI 行为层（无样式，与后端解耦）
    "neo-ui":            4,  # L4 UI 设计系统（品牌主题 + 组件）
    # GUI 宿主的共享内核驱动。依赖 neo-core(2)/neo-protocol(0)，与
    # neo-capability 同级 —— 是"真实依赖深度"的正确落点，不是特权层。
    # 它必须存在的原因：A3 禁止宿主互相依赖，而 egui 与 gpui 两个 GUI 宿主
    # 共用同一份驱动（里面守着"越界 Pump 不触发多余模型请求"这条要花钱的约束）。
    "neo-driver":        3,  # L3 共享驱动
    "neo-orchestration": 4,  # L4
    "neo-host-tui":      5,  # L5
    "neo-host-desktop":  5,  # L5
    "neo-host-egui":     5,  # L5（桌面原生 GUI：egui/eframe）
    "neo-host-gpui":     5,  # L5（桌面原生 GUI：GPUI / gpui-kit）
    "neo-host-web":      5,  # L5
    "neo-host-appserver": 5,  # L5（stdio JSON-RPC：编辑器/IDE 的通用入口）
    "neo-exec":          5,  # L5
    "neo-code-cli":      5,  # L5（bin 名 neo；crates.io 上 neo-cli 已被占用故包名加 code）
}
SIDE = {"neo-session", "neo-config"}   # 旁挂，任何层可用
HOSTS = {"neo-host-tui", "neo-host-desktop", "neo-host-egui", "neo-host-gpui",
         "neo-host-web", "neo-host-appserver", "neo-exec"}
# 注意 1 是混合层：既有 L1 PLATFORM（平台抽象：沙箱/剪贴板/通知），
# 也有 L1 BASE（宿主中立的文本语义）。两者依赖深度相同、互不依赖。
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
