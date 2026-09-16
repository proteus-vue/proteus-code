#!/usr/bin/env python3
"""
生成第三方许可证清单（THIRD-PARTY-LICENSES.md）。

# 它是什么，以及为什么需要

我们自己的代码用 Apache-2.0。依赖里的第三方包各有各的许可证，使用者在
审计时需要一个**能直接读的清单**，而不是自己跑一遍 `cargo metadata`。

`check_licenses.py` 回答的是"有没有混进不允许的许可证"（门禁问题），
本脚本回答的是"用了哪些、分别是什么许可"（交付物问题）。两者数据来源相同
但输出不同，所以分开。

# 覆盖范围：**可开源集的依赖图**，不是全仓

开源的只有那 5 个 crate，使用者拿到它们后会自己 `cargo build` 拉依赖 ——
他需要知道的是**这条依赖链**的许可，而不是本项目宿主（含 egui/wry）用了什么。
范围错了会让清单里出现一堆与他无关的条目，反而降低可用性。

# 关于 Apache-2.0 的 NOTICE 义务（§4(d)）

`§4(d)` 要求：若原作品带 `NOTICE` 文件，衍生分发必须保留其中的归属声明。
本脚本会**逐个检查依赖目录里是否真有 NOTICE 文件**并按实际情况标注 ——
实测当前提取集的 39 个 Apache-2.0 依赖**都没有 NOTICE**，因此该义务不触发。
把它写清楚是为了让读者知道"我们检查过"，而不是"我们漏了"。

（诚实边界：registry 里的目录是 cargo 解压出来的，未必含上游文件的全部。
若某包在上游带 NOTICE 而本地没有，本脚本会漏报。彻底的做法是用
`cargo-about`，它读的是包元数据而非本地目录。）

# 用法

    python3 scripts/gen_third_party_licenses.py            # 写到仓库根
    python3 scripts/gen_third_party_licenses.py --check    # 只校验是否最新（CI 用）
"""
import json
import os
import subprocess
import sys
from datetime import date

ROOT = os.environ.get("NEO_ROOT", os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))
OUT = os.path.join(ROOT, "THIRD-PARTY-LICENSES.md")
CHECK = "--check" in sys.argv[1:]


def load_ui_set():
    """可开源集同样从 UI 分层守卫读 —— 清单范围不在这里抄第二份。"""
    checks = os.path.join(ROOT, "docs", "neo-plan", "05-验证", "checks")
    sys.path.insert(0, checks)
    import check_ui_layering as layering

    return sorted(set(layering.UI_CRATES) | set(layering.UI_STACK_MAY_DEPEND_ON))


def reachable(meta, roots):
    nodes = {n["id"]: n for n in meta.get("resolve", {}).get("nodes", [])}
    id_of = {}
    for p in meta["packages"]:
        id_of.setdefault(p["name"], []).append(p["id"])
    seen, stack = set(), [i for r in roots for i in id_of.get(r, [])]
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for d in nodes.get(pid, {}).get("dependencies", []):
            stack.append(d)
    ws = {p["name"] for p in meta["packages"] if p.get("source") is None}
    return [p for p in meta["packages"] if p["id"] in seen and p["name"] not in ws]


def find_registry_dir(name, version=""):
    """在本地 registry 里找该包的目录（用于检查是否带 NOTICE）。

    ⚠️ 不能靠"按最后一个 `-` 切分"来还原包名：版本号里可能带
    build metadata 或预发布标签（`0.4.0+sdk-1.4.341.0`、
    `1.0.0-beta.1`），切出来的前缀不是包名 —— 实测因此把 spirv 误报成
    "未能检查"（它其实下载了）。
    正确做法：目录名必须**以 `包名-` 开头**，且优先精确匹配版本。
    """
    root = os.path.expanduser("~/.cargo/registry/src")
    if not os.path.isdir(root):
        return None
    hits = []
    for base in os.listdir(root):
        d = os.path.join(root, base)
        if not os.path.isdir(d):
            continue
        for entry in os.listdir(d):
            if entry == name:
                return os.path.join(d, entry)
            if entry.startswith(name + "-"):
                hits.append(os.path.join(d, entry))
    if not hits:
        return None
    if version:
        for h in hits:
            if os.path.basename(h) == f"{name}-{version}":
                return h
    return hits[0]


def has_notice(name, version=""):
    """包目录里是否带 NOTICE 文件。返回 True/False/None（目录不可用）。"""
    d = find_registry_dir(name, version)
    if not d:
        return None
    try:
        return any("notice" in f.lower() for f in os.listdir(d))
    except OSError:
        return None


def main():
    r = subprocess.run(
        ["cargo", "metadata", "--format-version", "1"],
        cwd=ROOT, capture_output=True, text=True,
    )
    if r.returncode != 0:
        print("❌ 无法读取 cargo metadata：", file=sys.stderr)
        print(r.stderr[-1200:], file=sys.stderr)
        return 1
    meta = json.loads(r.stdout)

    roots = load_ui_set()
    deps = sorted(reachable(meta, roots), key=lambda p: p["name"].lower())

    # 分类：按许可证表达式分组，组内按名字排序
    apache_hard, others = [], []
    for p in deps:
        lic = p.get("license") or "（未声明）"
        entry = (p["name"], p.get("version", ""), lic, p.get("repository") or "")
        # "必须用 Apache-2.0"的单独一组：这些是 §4(d) 检查对象
        if lic.strip() == "Apache-2.0" or lic.strip().startswith("Apache-2.0 AND"):
            apache_hard.append(entry)
        else:
            others.append(entry)

    lines = []
    lines.append("# 第三方许可证清单\n")
    lines.append(
        "本文件由 `scripts/gen_third_party_licenses.py` **自动生成**，请勿手工编辑。\n"
    )
    lines.append(f"生成日期：{date.today().isoformat()}\n")
    lines.append("## 覆盖范围\n")
    lines.append(
        f"本清单覆盖**可开源集**的完整依赖链，共 {len(deps)} 个第三方包。\n"
    )
    lines.append("可开源集（本仓库自有的、以 Apache-2.0 发布的 crate）：\n")
    for c in roots:
        lines.append(f"- `{c}`")
    lines.append(
        "\n范围限定在这条依赖链，是因为使用者拿到的是这几个 crate ——"
        "他需要知道的也是这一条链的许可情况，"
        "而本项目的宿主（含其它 GUI 后端）用到的依赖与他无关。\n"
    )

    lines.append("## 自身许可\n")
    lines.append(
        "可开源集的 crate 以 **Apache-2.0** 发布（各 crate 目录下有 `LICENSE` 全文）。\n"
    )

    lines.append("## Apache-2.0 依赖与 NOTICE 义务\n")
    lines.append(
        "Apache-2.0 第 4(d) 条要求：**若原作品带 `NOTICE` 文件**，衍生分发须保留"
        "其中的归属声明。下表逐个标注了实际检查结果 ——"
        "「无 NOTICE」表示该包**随包发布的文件里**没有 NOTICE，因此该项义务不触发。\n"
    )
    lines.append("| 包 | 版本 | 许可证 | NOTICE | 来源 |")
    lines.append("|---|---|---|---|---|")
    for name, ver, lic, repo in apache_hard:
        n = has_notice(name, ver)
        mark = {True: "**有**", False: "无", None: "未能检查"}[n]
        lines.append(f"| `{name}` | {ver} | {lic} | {mark} | {repo or '—'} |")
    lines.append("")
    if any(has_notice(n, v) for n, v, _, _ in apache_hard):
        lines.append(
            "⚠️ 上表中有标注为「**有**」的包 —— 必须把它们的 NOTICE 内容"
            "并入本仓库根的 `NOTICE` 文件。\n"
        )
    else:
        lines.append(
            "结论：当前**没有任何** Apache-2.0 依赖携带 NOTICE 文件，"
            "因此无需额外维护 `NOTICE`。若将来新增依赖带了 NOTICE，"
            "本脚本的输出会变，届时需补上。\n"
        )

    lines.append("## 其余依赖\n")
    lines.append("| 包 | 版本 | 许可证 | 来源 |")
    lines.append("|---|---|---|---|")
    for name, ver, lic, repo in others:
        lines.append(f"| `{name}` | {ver} | {lic} | {repo or '—'} |")
    lines.append("")

    lines.append("## 许可证全文\n")
    lines.append(
        "各依赖的许可证全文随包附带，可在 cargo registry 缓存中查看：\n\n"
        "```\n~/.cargo/registry/src/*/<包名>-<版本>/LICENSE*\n```\n\n"
        "本文件不复印全文：几十份许可证文本会让它难以阅读，而它们与"
        "crates.io 上的版本逐字节相同、随时可取。\n"
    )

    text = "\n".join(lines)

    if CHECK:
        if not os.path.isfile(OUT):
            print(f"❌ 缺少 {OUT} —— 请运行 scripts/gen_third_party_licenses.py")
            return 1
        cur = open(OUT).read()
        # 只比较"内容主体"，忽略生成日期行（否则每天都会判定为过期）
        def strip_date(t):
            return "\n".join(
                l for l in t.splitlines() if not l.startswith("生成日期：")
            )
        if strip_date(cur) != strip_date(text):
            print("❌ THIRD-PARTY-LICENSES.md 已过期（依赖有变动）")
            print("   请重新运行：python3 scripts/gen_third_party_licenses.py")
            return 1
        print("✅ THIRD-PARTY-LICENSES.md 是最新的")
        return 0

    with open(OUT, "w") as f:
        f.write(text)
    print(f"✅ 已写入 {OUT}")
    print(f"   第三方包 {len(deps)} 个（其中 Apache-2.0 硬依赖 {len(apache_hard)} 个）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
