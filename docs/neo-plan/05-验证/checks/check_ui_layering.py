#!/usr/bin/env python3
"""
UI 栈守卫 —— 保证「整目录复制出去就能开源」不是口号。

校验项：
  U1 UI 栈四层不依赖内核轴（可整目录搬走）
  U2 单一 GPUI pin：除 neo-ui-kit 外任何 crate 不得声明 gpui 系列依赖
  U3 UI 行为层不依赖门面层（backend 无关的最后一道缝）
  U4 UI 栈分层方向正确（kit < render < behavior < ui）

# 为什么需要这个门禁

`docs/gpui-方案/docs/00-架构总纲.md` 自己写了一句要害：

> 未来要开源时，希望是「复制两个目录出去就能建新仓」。
> 这些约束**现在就要做**（成本近乎为零，事后改成本极高）。

"事后改成本极高"这件事必须变成**机器可判**的，否则它只会是一句愿望 ——
因为**加一行依赖看起来永远是无害的**，而每加一行的代价要两年后才显现。
这就是本脚本存在的全部理由。

# 与 check_architecture.py 的分工

那个脚本守**内核轴**（L0–L5，防宿主反向污染内核）。
本脚本守**UI 轴的对外边界**（防 UI 栈被内核轴粘住）。
两条轴正交，所以规则分开写、各自独立可读。
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.environ.get(
    "NEO_ROOT", os.path.join(HERE, "..", "..", "..", "..")
)

# ── UI 栈四层：将来要**整目录复制出去开源**的部分 ──────────────────────
#
# 键 = 目录名，值 = 允许依赖的其他 UI 栈 crate（分层方向：只能向下）。
UI_CRATES = {
    "neo-ui-kit":      [],                      # L1 门面（唯一 pin GPUI 的地方）
    "neo-ui-render":   ["neo-ui-kit"],          # L2 渲染缝
    "neo-ui-behavior": ["neo-ui-render"],       # L3 行为层（**刻意不依赖 kit**）
    "neo-ui":          ["neo-ui-behavior", "neo-ui-render", "neo-ui-kit"],  # L4 设计系统
}

# UI 栈可以依赖的**仓内非 UI** crate —— 白名单，加一项要写明理由。
#
# `neo-text` 是宿主中立的文本语义（Tone / Palette / Markdown / 高亮），
# 它本身不依赖任何 GUI。UI 栈依赖它是**有意的**：这样"同一个 Tone 在
# TUI / egui / gpui 里是同一个颜色"有单一来源。
#
# ⚠️ 但这也意味着**开源时 `neo-text` 要一并带走**（或让 neo-ui 自带主题定义）。
# 这个取舍在 docs/gpui-方案 的阶段划分里尚未决定，所以这里显式列出来 ——
# 让它始终可见，而不是藏在某条 import 里。
UI_STACK_MAY_DEPEND_ON = {"neo-text"}

# 除门面层外，任何 crate 都不得声明这些依赖（单一 pin）
GPUI_PACKAGES = {
    "gpui", "gpui-pre", "gpui_platform", "gpui-pre-platform",
    "gpui-kit", "gpui-component", "gpui-base", "gpui-kit-assets",
}
# 唯一允许声明它们的 crate
GPUI_PIN_OWNER = "neo-ui-kit"


def parse_cargo(path):
    """极简 TOML 解析：够用于本仓库自写的 Cargo.toml。

    返回 (name, deps)：deps 是 [(crate名, 原文行)]，**含注释行在内**，
    因为我们既要看路径依赖（分层）也要看包名依赖（单一 pin）。
    """
    name = None
    deps = []
    section = None
    for raw in open(path, encoding="utf-8"):
        line = raw.split("#")[0].strip()   # 去注释后再解析
        if not line:
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
        elif section in ("dependencies", "dev-dependencies", "build-dependencies"):
            deps.append((k, v))
    return name, deps


def path_dep_name(value):
    """从 `{ path = "../x", ... }` 取出 x（末段目录名）。"""
    m = re.search(r'path\s*=\s*"[^"]*/([A-Za-z0-9_\-]+)"', value)
    return m.group(1) if m else None


def declared_package_name(key, value):
    """从 `gpui = { package = "gpui-pre", ... }` 取出真实包名；否则用键名。"""
    m = re.search(r'package\s*=\s*"([^"]+)"', value)
    return m.group(1) if m else key


def main():
    failures, checks = [], []

    members = []
    root_cargo = os.path.join(ROOT, "Cargo.toml")
    in_members = False
    for raw in open(root_cargo, encoding="utf-8"):
        line = raw.split("#")[0].strip()
        if line.startswith("members"):
            in_members = True
            continue
        if in_members:
            if line.startswith("]"):
                break
            m = re.match(r'"([^"]+)"', line)
            if m:
                members.append(m.group(1))

    graph = {}       # crate -> [依赖的仓内 crate]
    pkgs = {}        # crate -> [(声明名/包名, 原文)]
    for m in members:
        p = os.path.join(ROOT, m, "Cargo.toml")
        if not os.path.isfile(p):
            continue
        n, deps = parse_cargo(p)
        if not n:
            continue
        graph[n] = [d for d, v in deps if path_dep_name(v)]
        pkgs[n] = deps

    # ── U1 UI 栈不依赖内核轴 ────────────────────────────────────────────
    for crate in UI_CRATES:
        if crate not in graph:
            failures.append(f"U1 声明的 UI 栈 crate 不存在: {crate}")
            continue
        for d in graph[crate]:
            if d in UI_CRATES or d in UI_STACK_MAY_DEPEND_ON:
                continue
            failures.append(
                f"U1 {crate} 依赖了内核轴 crate '{d}' —— "
                f"UI 栈必须能整目录复制出去开源，被内核粘住就搬不动了"
            )
    checks.append(("U1", "UI 栈不依赖内核轴（可整目录搬走）",
                   not any(f.startswith("U1") for f in failures)))

    # ── U2 单一 GPUI pin ────────────────────────────────────────────────
    for crate, deps in sorted(pkgs.items()):
        for key, val in deps:
            pkg = declared_package_name(key, val)
            if pkg not in GPUI_PACKAGES:
                continue
            if crate == GPUI_PIN_OWNER:
                continue
            failures.append(
                f"U2 {crate} 声明了 GPUI 依赖 '{pkg}' —— "
                f"只允许 {GPUI_PIN_OWNER} 声明；其余一律 use neo_ui_kit::gpui::*。"
                f"（两份 gpui 会让 Cargo 编译两份引擎，报出类型身份分裂的怪错误）"
            )
    checks.append(("U2", f"单一 GPUI pin（仅 {GPUI_PIN_OWNER} 可声明）",
                   not any(f.startswith("U2") for f in failures)))

    # ── U3 行为层不依赖门面层 ──────────────────────────────────────────
    #
    # 这条是"渲染后端可替换"的最后一道缝：行为层若能碰 GPUI，
    # 它就绑死在当前后端上了。
    be_deps = graph.get("neo-ui-behavior", [])
    bad = [d for d in be_deps if d in ("neo-ui-kit", "neo-ui")]
    for d in bad:
        failures.append(
            f"U3 neo-ui-behavior 依赖了 {d} —— 行为层必须与渲染后端无关"
            f"（它装的是焦点仲裁/按键路由这类去掉颜色尺寸仍然成立的东西）"
        )
    checks.append(("U3", "UI 行为层与渲染后端解耦", not bad))

    # ── U4 分层方向：只能向下 ──────────────────────────────────────────
    order = ["neo-ui-kit", "neo-ui-render", "neo-ui-behavior", "neo-ui"]
    rank = {c: i for i, c in enumerate(order)}
    for crate, deps in graph.items():
        if crate not in rank:
            continue
        for d in deps:
            if d not in rank:
                continue
            if rank[d] > rank[crate]:
                failures.append(
                    f"U4 向上依赖：{crate}(L{rank[crate]+1}) -> {d}(L{rank[d]+1})"
                )
    checks.append(("U4", "UI 栈分层方向正确（只向下）",
                   not any(f.startswith("U4") for f in failures)))

    print("=" * 62)
    print("UI 栈守卫 · 开源边界校验")
    print("=" * 62)
    print("  UI 栈分层（将来可整目录开源的部分）：")
    for c in order:
        deps = ", ".join(graph.get(c, [])) or "-"
        mark = "" if c in graph else "   ← 尚未创建"
        print(f"    L{rank[c]+1}  {c:18} -> {deps}{mark}")
    print("-" * 62)
    for code, desc, ok in checks:
        print(f"  [{'PASS' if ok else 'FAIL'}] {code}  {desc}")
    print("-" * 62)
    if failures:
        print(f"❌ UI 栈守卫失败，共 {len(failures)} 项：")
        for f in failures:
            print("   -", f)
        return 1
    print("✅ UI 栈守卫通过：开源边界完整、单一 pin、行为层与后端解耦")
    return 0


if __name__ == "__main__":
    sys.exit(main())
