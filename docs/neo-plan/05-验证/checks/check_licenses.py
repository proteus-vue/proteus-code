#!/usr/bin/env python3
"""
许可证合规守卫 —— 把"我们只用宽松许可的依赖"变成机器可判的事实。

数据来源：`cargo metadata`。**不依赖 cargo-deny**：那个工具需要单独安装、
且联网拉 advisory 库，而这台机器 / CI 上未必有。本脚本只用 cargo 自带的
能力 + Python 标准库，因此"一定能跑"。

# 为什么需要它

"我们只用宽松许可的依赖"是一句**会腐烂**的话：任何一次 `cargo add` 都可能
拖进一个 GPL 依赖，而 cargo 不会提醒你，编译也不报错。等到真要发布时才发现，
换依赖的成本远高于现在加一条检查。

这与本仓其它守卫（依赖方向、单一 pin、可提取性）同一个思路：
**把口头约定变成机器可判**。

# 判据不是"法律上能不能用"，而是"会不会给使用者带来义务或风险"

- 宽松许可（MIT / Apache-2.0 / BSD / ISC / Zlib / Unicode）：允许
- 弱 copyleft（MPL-2.0）：允许（文件级，不改它就没有额外义务）
- 强 copyleft（GPL / AGPL）：拒绝 —— 会吓跑商业使用者，
  而本项目的开源目标是拿采用率，不是拿传染性

# SPDX 表达式的语义（这是最容易搞错的部分）

`A OR B`  → **任一**满足即可（使用者自己选，所以 `MIT OR GPL-3.0` 是宽松的）
`A AND B` → **全部**都要满足
`A WITH E` → 带例外的整体（`Apache-2.0 WITH LLVM-exception` 是一个标识符）

另有一种**老式写法**用斜杠：`MIT/Apache-2.0`、`Apache-2.0/MIT`。
它与 `MIT OR Apache-2.0` 同义。把斜杠语义搞错会让检查得出完全相反的结论 ——
第一版实现就因为正则写错，把 59 个正常依赖误判为违规。

# 诚实边界

- 它检查的是 **Cargo.toml 里声明的**许可证，不做许可证文本比对
  （那需要 cargo-deny / cargo-about 的完整能力）。
  声明与实际文本不符是"上游写错了"，不是本仓能防的。
- 它覆盖全仓依赖图（含宿主与即将删除的 egui）。若要只查"待开源集"，
  见 `--extract-only`。
- advisory（已知漏洞）检查**不在**这里：那需要联网拉数据库，
  放进 CI 的独立步骤更合适（见 deny.toml）。
"""
import json
import os
import re
import subprocess
import sys

ROOT = os.environ.get("NEO_ROOT", ".")

# 允许的许可证标识符。与 deny.toml 的 `allow` 列表保持一致 ——
# 两处都是"可判定的声明"，但用途不同：cargo-deny 用于 CI 的完整检查，
# 本脚本用于"没有 cargo-deny 也能跑"的兜底。**改一处要改两处**，
# 所以两边都写了指向对方的注释。
ALLOWED = {
    "MIT",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Zlib",
    "0BSD",
    "Unicode-3.0",
    "BSL-1.0",
    "CC0-1.0",
    "Unlicense",
    "bzip2-1.0.6",
    "MPL-2.0",
    "OFL-1.1",
    "Ubuntu-font-1.0",
    "NCSA",
    "MIT-0",
}


def _balanced(s):
    depth = 0
    for c in s:
        if c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth < 0:
                return False
    return depth == 0


def _split_top(s, sep):
    """在括号深度 0 处按 `sep` 切分（不切进括号里）。"""
    out, depth, i, last = [], 0, 0, 0
    while i < len(s):
        c = s[i]
        if c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
        if depth == 0 and s.startswith(sep, i):
            out.append(s[last:i])
            last = i + len(sep)
            i = last
            continue
        i += 1
    out.append(s[last:])
    return [p.strip() for p in out if p.strip()]


def parse(expr):
    """SPDX 表达式 → 嵌套树。返回 (kind, value)，kind ∈ {OR, AND, ATOM}。"""
    # 老式斜杠写法 `MIT/Apache-2.0` 与 `MIT OR Apache-2.0` 同义。
    # 斜杠可以直接替换：许可证标识符里不含斜杠。
    expr = expr.replace("/", " OR ").strip()
    while expr.startswith("(") and expr.endswith(")") and _balanced(expr[1:-1]):
        expr = expr[1:-1].strip()
    for op, sep in (("OR", " OR "), ("AND", " AND ")):
        parts = _split_top(expr, sep)
        if len(parts) > 1:
            return (op, [parse(p) for p in parts])
    # WITH 是整体：`Apache-2.0 WITH LLVM-exception` 是一个标识符
    parts = _split_top(expr, " WITH ")
    if len(parts) > 1:
        return ("ATOM", " WITH ".join(parts))
    return ("ATOM", expr.strip())


def accepted(node):
    kind, value = node
    if kind == "OR":
        return any(accepted(v) for v in value)
    if kind == "AND":
        return all(accepted(v) for v in value)
    return value in ALLOWED


def workspace_names(meta):
    return {p["name"] for p in meta["packages"] if p.get("source") is None}


def reachable(meta, roots):
    """从 roots 出发可达的第三方包名集合。"""
    by_name = {}
    for p in meta["packages"]:
        by_name.setdefault(p["name"], []).append(p)
    ws = workspace_names(meta)
    # resolve 里的依赖图按 package id 索引
    nodes = {n["id"]: n for n in meta.get("resolve", {}).get("nodes", [])}
    id_of = {}
    for p in meta["packages"]:
        id_of.setdefault(p["name"], []).append(p["id"])

    seen, stack = set(), []
    for r in roots:
        for pid in id_of.get(r, []):
            stack.append(pid)
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes.get(pid, {}).get("dependencies", []):
            stack.append(dep)
    out = set()
    for n in meta["packages"]:
        if n["id"] in seen and n["name"] not in ws:
            out.add(n["name"])
    return out


def main():
    extract_only = "--extract-only" in sys.argv[1:]
    r = subprocess.run(
        ["cargo", "metadata", "--format-version", "1"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if r.returncode != 0:
        print("❌ 无法读取 cargo metadata：")
        print(r.stderr[-1500:])
        return 1
    meta = json.loads(r.stdout)

    scope = "全仓依赖图"
    targets = None
    if extract_only:
        # 可开源集同样从 UI 分层守卫读，不抄第二份清单
        here = os.path.dirname(os.path.abspath(__file__))
        sys.path.insert(0, here)
        try:
            import check_ui_layering as layering

            roots = sorted(set(layering.UI_CRATES) | set(layering.UI_STACK_MAY_DEPEND_ON))
            targets = reachable(meta, roots)
            scope = f"可开源集（{', '.join(roots)}）的依赖图"
        except Exception as e:
            print(f"❌ 无法读到可开源集：{e}")
            return 1

    print("=" * 62)
    print("许可证合规守卫")
    print("=" * 62)
    print(f"  范围：{scope}")
    print(f"  允许的许可证：{len(ALLOWED)} 种（全部为宽松许可，见脚本头说明）")
    print("-" * 62)

    ws = workspace_names(meta)
    bad, no_field, checked = [], [], 0
    for p in meta["packages"]:
        if p.get("source") is None:
            continue
        if targets is not None and p["name"] not in targets:
            continue
        checked += 1
        lic = p.get("license")
        if not lic:
            no_field.append(p["name"])
            continue
        if not accepted(parse(lic)):
            bad.append((p["name"], lic))

    print(f"  检查了 {checked} 个第三方包（本仓 {len(ws)} 个私有 crate 不查）")
    print("-" * 62)

    failures = []
    if bad:
        # 按表达式归类：同一表达式通常是一类依赖
        grouped = {}
        for name, lic in bad:
            grouped.setdefault(lic, []).append(name)
        for lic, names in sorted(grouped.items()):
            failures.append(
                f"L1 含不允许的许可证：{lic}\n      包：{', '.join(sorted(names)[:8])}"
                + (" …" if len(names) > 8 else "")
            )
    if no_field:
        failures.append(
            "L2 这些包没有声明许可证，机器无法判断："
            + ", ".join(sorted(no_field)[:10])
            + (f" …（共 {len(no_field)} 个）" if len(no_field) > 10 else "")
            + "\n      缺声明可能是'作者没想过'（可用）也可能是'有专门限制'（不可用），"
            + "机器分辨不了 —— 请人工看过再决定是否加白名单。"
        )

    if failures:
        print(f"❌ 许可证守卫失败，共 {len(failures)} 项：")
        for f in failures:
            print("   -", f)
        return 1

    print("✅ 许可证守卫通过：声明的许可证全部在允许列表内")
    if extract_only:
        print("   （仅覆盖可开源集；发布前请用 cargo deny check licenses 跑全量）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
