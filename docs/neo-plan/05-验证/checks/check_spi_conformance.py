#!/usr/bin/env python3
"""
SPI 合规守卫 —— 强制 Proteus 的「一个 seam 三个角色 + ≥2 后端」纪律。

方法论依据：`02-架构设计/Proteus方法论-语义核心与后端SPI.md`

为什么需要这个检查：
  DSH 的问题不是"插件太多"，而是 **seam 没有名字、没有门禁**。
  约 90 个 workspace 包 = 90 个无名 seam，无法静态分析、无法验证可替换性。
  NEO 把 seam 收敛为 5 个**有名**的 SPI，并要求每个都有 ≥2 个真实后端 ——
  否则"可替换"只是未经验证的宣称（Proteus 称之为「假 SPI」）。

校验项：
  S1 声明的 SPI 必须都能在源码中找到契约定义（trait）
  S2 每个 SPI 必须 >= 2 个后端实现（杜绝假 SPI）
  S3 每个 SPI 必须有 conformance 断言（对应 check 或铁律编号）
  S4 SPI 总数不得膨胀（超过阈值说明又在走 DSH 的老路）
  S5 宿主后端数必须 >= 2（T6 语义等价的前提）
"""
import os, re, sys, json
from collections import defaultdict

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.environ.get("NEO_ROOT", os.path.join(HERE, "..", "..", "..", ".."))

# 允许的 SPI 上限。超过它说明 seam 又在无序增生 —— 这正是 DSH 的病。
MAX_SPI = 8

# 声明的 5 个 SPI：契约 trait、后端、对应铁律、落地状态。
#
# status 的语义（这是本检查诚实性的关键）：
#   "landed"  —— 后端已真实存在，强制 >= 2，不满足即 FAIL
#   "planned" —— 契约已定、后端待实现，报为待办（WARN），不推翻整个门禁
#
# 这样做是因为：把未落地的 SPI 也按 FAIL 处理，会让门禁永远红、进而被忽略；
# 而把它们当通过，又是在说谎。**分开报，既守住已宣称的部分，也不隐瞒缺口。**
SPI_SPEC = {
    "ModelProvider": {
        "trait": "ModelProvider",
        "conformance": "T2",
        "test": "crates/neo-mock/tests/conformance.rs",
        "status": "landed",
    },
    "SandboxBackend": {
        "trait": "SandboxBackend",
        "conformance": "T4",
        "test": "crates/neo-mock/tests/conformance.rs",
        "status": "landed",
    },
    "SessionPersistence": {
        "trait": "SessionPersistence",
        "conformance": "T3",
        "test": "crates/neo-mock/tests/conformance.rs",
        "status": "landed",
    },
    "HostBackend": {
        "trait": "HostBackend",
        "conformance": "T6",
        "test": "crates/neo-mock/tests/conformance.rs",
        "status": "landed",
    },
    "ToolTransport": {
        "trait": "Tool",
        "conformance": "T5",
        "test": "crates/neo-mock/tests/conformance.rs",
        "status": "landed",
    },
}


def read(path):
    try:
        with open(path, encoding="utf-8") as f:
            return f.read()
    except OSError:
        return ""


def collect_sources():
    """全部 crate 源码，用于契约与后端存在的搜索。"""
    out = {}
    crates = os.path.join(ROOT, "crates")
    for name in sorted(os.listdir(crates)):
        src = os.path.join(crates, name, "src")
        if not os.path.isdir(src):
            continue
        text = "\n".join(read(os.path.join(src, f)) for f in sorted(os.listdir(src)) if f.endswith(".rs"))
        out[name] = text
    return out


def run():
    srcs = collect_sources()
    corpus = "\n".join(srcs.values())
    problems = []
    summary = []

    # ---- S1 契约必须存在 ------------------------------------------------
    for spi, spec in SPI_SPEC.items():
        trait = spec["trait"]
        if trait is None:
            continue
        # 契约必须被声明为 trait（本仓库约定：trait 名 == SPI 名或承担其角色）
        if not re.search(rf"\btrait\s+{re.escape(trait)}\b", corpus):
            problems.append(f"S1 {spi}: 未找到契约声明 `trait {trait}`")
        else:
            summary.append(f"  ✓ {spi} 契约 trait `{trait}` 存在")

    # ---- S2 每个 SPI >= 2 个**真实实现**（杜绝假 SPI）-------------------
    #
    # 这是本检查的核心修正。首版按「源码里是否出现后端名」计数，结果
    # HostBackend 在只有一个实现时也报 4 个后端 —— 因为 tui/web/exec
    # 只出现在注释里。**假门禁给假保证，比没有门禁更糟。**
    # 现在只认 `impl <Trait> for` 这个语法事实。
    impl_re = lambda t: re.compile(rf"^\s*impl(?:<[^>]*>)?\s+{re.escape(t)}\s+for\s+", re.M)
    pending = []
    for spi, spec in SPI_SPEC.items():
        trait = spec["trait"]
        if trait is None:
            continue
        total, per_crate = 0, []
        for crate, text in srcs.items():
            k = len(impl_re(trait).findall(text))
            if k:
                total += k
                per_crate.append(f"{crate}({k})")
        if total >= 2:
            summary.append(f"  ✓ {spi} (`{trait}`): {total} 个真实实现 — {', '.join(sorted(per_crate))}")
        elif spec.get("status") == "landed":
            problems.append(
                f"S2 {spi} 已宣称 landed，但只找到 {total} 个 `impl {trait} for` "
                f"({', '.join(per_crate) or '无'}) —— 假 SPI，可替换性从未被验证（AP-01）"
            )
        else:
            pending.append(f"{spi}: 待实现（当前 {total} 个 impl）")

    # ---- S2b landed SPI 必须有 conformance 测试文件 --------------------
    for spi, spec in SPI_SPEC.items():
        if spec.get("status") != "landed":
            continue
        rel = spec.get("test")
        path = os.path.join(ROOT, rel) if rel else None
        if not (path and os.path.isfile(path)):
            problems.append(f"S2b {spi} 已宣称 landed，但 conformance 测试缺失（{rel}）—— 无契约测试的抽象是假 SPI（AP-03）")
        else:
            # 测试文件里应能看到该 SPI 的 **trait 名**，避免"文件存在但没测它"。
            # 用 trait 名而非 SPI id 判定：id 是人取的（"ToolTransport"），
            # trait 名才是代码事实（"Tool"）—— 首版用 id 判定产生了假阴性。
            body = open(path, encoding="utf-8").read()
            trait = spec["trait"]
            if not re.search(rf"\b{re.escape(trait)}\b", body):
                problems.append(f"S2b {spi}: conformance 文件存在但未见 `{trait}` 的契约用例")
            else:
                summary.append(f"  ✓ {spi} conformance 用例存在且含 `{trait}` 契约")

    # ---- S2c SPI 版本漂移检测（契约口径变化须改版本号）------------------
    import hashlib
    digests = {}
    for spi, spec in SPI_SPEC.items():
        t = spec["trait"]
        if not t:
            continue
        # 取契约 trait 的声明体作为口径指纹
        m = re.search(rf"pub trait {re.escape(t)}[^{{]*\{{(.*?)\n\}}", corpus, re.S)
        if m:
            digests[spi] = hashlib.sha256(m.group(1).encode()).hexdigest()[:12]
    stamp_path = os.path.join(HERE, "spi_contract_digests.json")
    if os.path.isfile(stamp_path):
        with open(stamp_path, encoding="utf-8") as f:
            recorded = json.load(f)
        for spi, d in digests.items():
            if spi in recorded and recorded[spi] != d:
                problems.append(
                    f"S2c {spi} 契约口径已变（{recorded[spi]} -> {d}）："
                    f"**必须显式确认这是有意变更，并更新 spi_contract_digests.json**。"
                    f"契约静默漂移是 SPI 最常见的隐性腐化。"
                )
        new_spis = [k for k in digests if k not in recorded]
        if new_spis:
            summary.append(f"  · 新增 SPI 待登记指纹：{', '.join(new_spis)}")
    else:
        summary.append("  · 无指纹基线（首次运行不校验漂移）")
    with open(stamp_path, "w", encoding="utf-8") as f:
        json.dump(digests, f, indent=2, sort_keys=True)

    # ---- S4 SPI 总数不得膨胀 -------------------------------------------
    n = len(SPI_SPEC)
    if n > MAX_SPI:
        problems.append(f"S4 SPI 总数 {n} 超过上限 {MAX_SPI} —— seam 无序增生是 DSH 的老路")
    else:
        summary.append(f"  ✓ SPI 总数 {n} <= {MAX_SPI}")

    # ---- S5 宿主后端 >= 2（T6 前提）------------------------------------
    hosts = [c for c in srcs if c.startswith("neo-host-")]
    if len(hosts) < 2:
        problems.append(f"S5 宿主后端只有 {len(hosts)} 个（{hosts}）—— T6 语义等价无从验证")
    else:
        summary.append(f"  ✓ 宿主后端 {len(hosts)} 个：{', '.join(sorted(hosts))}")

    # ---- S5b 宿主不得把 Electron/Chromium 作为**依赖**引入 ----------------
    # 这是本项目的核心取舍（系统 webview 替代 Electron），必须机器守住。
    # 注意：只在「依赖声明」与「use 语句」里判定 —— 注释里写"不依赖 Electron"
    # 是说明，不是违规。首版检查在此处产生过假阳性。
    banned = ["electron", "tauri", "chromium"]
    dep_re = re.compile(r"^\s*([A-Za-z0-9_-]+)\s*=", re.M)
    for host in hosts:
        crate_toml = read(os.path.join(ROOT, "crates", host, "Cargo.toml"))
        deps = {m.group(1).lower() for m in dep_re.finditer(crate_toml)}
        # use 语句（真实的代码依赖）
        uses = {u.lower() for u in re.findall(r"\buse\s+([a-z_][a-z0-9_]*)", srcs.get(host, ""), re.I)}
        for b in banned:
            if b in deps or b in uses:
                problems.append(
                    f"S5 {host}: 引入被禁依赖 `{b}` —— 桌面宿主必须基于系统 webview"
                )

    print("SPI 合规守卫（Proteus 方法论：seam 要少、要有名字、要有门禁）")
    print(f"  契约搜索根: {ROOT}")
    print(f"  声明的 SPI: {n}")
    print()
    for line in summary:
        print(line)
    print()

    if pending:
        print("待办（已声明但未落地；不计为失败，但必须被看见）：")
        for p in pending:
            print(f"  · {p}")
        print()

    if problems:
        print("发现问题：")
        for p in problems:
            print(f"  ✗ {p}")
        return 1

    print("  ✅ 每个声明 seam 都有契约、>=2 后端、conformance 关联；SPI 数量未膨胀")
    return 0


if __name__ == "__main__":
    sys.exit(run())
