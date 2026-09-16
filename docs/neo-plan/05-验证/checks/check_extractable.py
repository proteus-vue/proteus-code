#!/usr/bin/env python3
"""
可提取性守卫 —— 让「复制几个目录出去就能建新仓」变成**每次都能验**的事实。

校验方式：真的把待开源 crate **复制到一个空目录**，照"新仓作者会写的"
最小 workspace 根合成一份 Cargo.toml，然后让 cargo 自己去解析它。

# 为什么需要它（与 check_ui_layering.py 的分工）

`check_ui_layering.py` 守的是 **Cargo.toml 里的依赖声明**：
不许依赖内核轴、不许两个 crate 各 pin 一份 gpui、分层方向只能向下。

但它守不住这三类"声明看起来完全合法、搬走却编不过"的情况：

1. **路径依赖逃出提取集**。`neo-ui` 里写 `neo-core = { path = "../neo-core" }`
   是合法声明（分层守卫会先拦，但它拦的是"依赖内核轴"这个**理由**，
   不是"搬走会断"这个**事实**）。一旦有人把白名单放宽，卡住的就不是守卫而是新用户。
2. **workspace 继承键在新仓根里不存在**。crate 里写
   `gpui-kit = { workspace = true }` 时，新仓作者必须自己在根
   `[workspace.dependencies]` 里写上它。漏了会得到
   "failed to parse manifest" —— 而这条错误**只在真正搬走时**才出现。
3. **提取集不闭合**。某个 crate 引用了一个既不在提取集、也不在
   crates.io 上的包（本仓独有的私有 crate）。

这三类的共同点：**主仓里永远看不出来**。主仓有全部成员、全部依赖表，
它们是"在别人的机器上"才会炸的那种问题 —— 也正因如此，
只靠"我们在 CI 里编过了"给不出任何保证。

# 为什么用 `cargo metadata --no-deps` 而不是 `cargo check`

实测（本机）：`cargo metadata --no-deps` 对上面 1/2 两类都会**直接报错**
（"failed to load manifest for workspace member..."），而耗时 **0.1 秒**。
`cargo check` 要做同样的事要 1.5 分钟（gpui-kit 编译）。

一个能跑进每次门禁的 0.1 秒检查，比一个"应该跑但太慢所以没人跑"的
1.5 分钟检查有用得多。**真正完整编译**的演练留在这里的 `--full`：
它慢，但那才是方案 DoD 里那条"实际演练一次"，发布前 / CI 该跑。

# 诚实边界

- `--full` 默认**不开**：门禁跑的是结构检查。`scripts/verify.sh` 不传 `--full`。
- 它验的是"**编译得起来**"。验不了"这个 crate 的文档口吻是否独立"、
  "README 是否齐"—— 那些是发布纪律，不是可编译性（见 Phase 3 清单）。
- 它不验证 crates.io 上否真的存在这些版本（不联网）：`--no-deps` +
  离线解析。真要发布还得 `cargo publish --dry-run`，那是另一道门。
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.environ.get("NEO_ROOT", os.path.join(HERE, "..", "..", "..", ".."))
CRATES = os.path.join(ROOT, "crates")

# 「哪些 crate 属于可开源集」只有**一处定义** —— 从分层守卫里读，
# 不在这里抄一遍。抄一遍的下场是两份清单漂移：加了新 crate 只改了其中一处，
# 于是这个守卫要么漏检、要么报一个根本不存在的失败。
try:
    sys.path.insert(0, HERE)
    import check_ui_layering as layering  # noqa: E402

    EXTRACT_SET = sorted(set(layering.UI_CRATES) | set(layering.UI_STACK_MAY_DEPEND_ON))
except Exception as e:  # pragma: no cover - 配置读不到时要说得清楚
    print(f"❌ 无法从 check_ui_layering.py 读到可开源集：{e}")
    print("   （这个守卫刻意不自己维护一份清单，避免两份漂移）")
    sys.exit(1)


def read_manifest(path):
    """读 Cargo.toml。需要 tomllib（Python ≥3.11）。"""
    import tomllib

    with open(path, "rb") as f:
        return tomllib.load(f)


def workspace_inherited_keys(manifest):
    """找出 crate 里所有 `xxx.workspace = true` 引用的键。

    分两类：`[workspace.package]` 里的（版本/许可证等元数据）与
    `[workspace.dependencies]` 里的（依赖）。新仓作者两类都要补上，
    所以这里都要收集。
    """
    pkg_keys, dep_keys = set(), set()
    # 元数据键（继承后直接是字符串），可能在 [package] 里
    for k, v in manifest.get("package", {}).items():
        if isinstance(v, dict) and v.get("workspace") is True:
            pkg_keys.add(k)
    # 依赖键：出现在 dependencies / dev-dependencies / build-dependencies 下
    for section in ("dependencies", "dev-dependencies", "build-dependencies"):
        for k, v in manifest.get(section, {}).items():
            if isinstance(v, dict) and v.get("workspace") is True:
                dep_keys.add(k)
    # target 下的依赖也算（跨平台构建会用到同一条根表）
    for _tgt, tval in manifest.get("target", {}).items():
        if not isinstance(tval, dict):
            continue
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            for k, v in tval.get(section, {}).items():
                if isinstance(v, dict) and v.get("workspace") is True:
                    dep_keys.add(k)
    return pkg_keys, dep_keys


def path_deps(manifest):
    """找出 crate 声明里所有 `path = "..."` 依赖及其名字。"""
    out = []
    for section in ("dependencies", "dev-dependencies", "build-dependencies"):
        for k, v in manifest.get(section, {}).items():
            if isinstance(v, dict) and "path" in v:
                out.append((k, v["path"]))
    for _tgt, tval in manifest.get("target", {}).items():
        if not isinstance(tval, dict):
            continue
        for section in ("dependencies", "dev-dependencies", "build-dependencies"):
            for k, v in tval.get(section, {}).items():
                if isinstance(v, dict) and "path" in v:
                    out.append((k, v["path"]))
    return out


def toml_str(s):
    return '"' + str(s).replace("\\", "\\\\").replace('"', '\\"') + '"'


def main():
    full = "--full" in sys.argv[1:]

    print("=" * 62)
    print("可提取性守卫 · 复制到空仓能否编译")
    print("=" * 62)
    print(f"  可开源集（{len(EXTRACT_SET)} 个 crate，来自 check_ui_layering.py）：")
    print("   ", ", ".join(EXTRACT_SET))
    print("-" * 62)

    failures = []

    # ── 1. 提取集必须都存在 ──────────────────────────────────────────────
    missing = [c for c in EXTRACT_SET if not os.path.isdir(os.path.join(CRATES, c))]
    if missing:
        failures.append(f"E1 可开源集里的 crate 不存在：{', '.join(missing)}")

    # ── 2. 路径依赖不能逃出提取集 ────────────────────────────────────────
    #
    # 这一条本可以交给 cargo 去撞（它也会报错），但自己先查一次的好处是
    # **错误信息说得出"逃到哪里去了"** —— cargo 只说某个 manifest 解析失败，
    # 不告诉你是哪个 path 依赖越了界。
    escaped = []
    pkg_keys_needed, dep_keys_needed = set(), set()
    for c in EXTRACT_SET:
        mf = os.path.join(CRATES, c, "Cargo.toml")
        if not os.path.isfile(mf):
            continue
        man = read_manifest(mf)
        pk, dk = workspace_inherited_keys(man)
        pkg_keys_needed |= pk
        dep_keys_needed |= dk
        for name, path in path_deps(man):
            # 归一化后看目标是不是提取集里的某个 crate
            target = os.path.normpath(os.path.join(CRATES, c, path))
            ok = any(target == os.path.normpath(os.path.join(CRATES, x)) for x in EXTRACT_SET)
            if not ok:
                escaped.append(f"{c} -> {name} (path={path})")
    if escaped:
        failures.append(
            "E2 路径依赖逃出可开源集（复制出去会解析失败）：" + "; ".join(escaped)
        )

    # ── 3. 合成空仓根，让 cargo 真的解析一次 ─────────────────────────────
    root_man = read_manifest(os.path.join(ROOT, "Cargo.toml"))
    ws_pkg = root_man.get("workspace", {}).get("package", {})
    ws_deps = root_man.get("workspace", {}).get("dependencies", {})

    # 新仓作者要补的元数据键：直接取主仓的 [workspace.package]。
    # 这些是**元信息**（版本/许可证/仓库地址），照搬是合理的；
    # 真正要检查的是下面那条：依赖表里有没有它需要的条目。
    pkg_lines = []
    for k in sorted(pkg_keys_needed):
        if k not in ws_pkg:
            failures.append(
                f"E3 crate 继承了 workspace.package.{k}，但主仓根里没有这一项 —— "
                f"新仓作者无从复制"
            )
            continue
        v = ws_pkg[k]
        if isinstance(v, list):
            items = ", ".join(toml_str(x) for x in v)
            pkg_lines.append(f"{k} = [{items}]")
        else:
            pkg_lines.append(f"{k} = {toml_str(v)}")

    dep_lines = []
    for k in sorted(dep_keys_needed):
        if k not in ws_deps:
            failures.append(
                f"E4 crate 依赖 {k} 走 workspace 继承，但主仓根 "
                f"[workspace.dependencies] 里没有它 —— 新仓作者无从复制"
            )
            continue
        spec = ws_deps[k]
        # 只接受"纯版本"或"带版本的外部依赖"。若它是 path 依赖，
        # 那就是 E2 那类问题（逃出提取集），这里再说一次是因为
        # 它的表现形式不同（声明处看起来只是个键名）。
        if isinstance(spec, dict) and "path" in spec and "version" not in spec:
            failures.append(
                f"E5 {k} 在主仓根里是**纯 path 依赖**（{spec['path']}）—— "
                f"新仓拿不到它，必须在发布前改成带 version 的依赖"
            )
            continue
        dep_lines.append(f"{k} = {_dep_to_toml(spec)}")

    if failures:
        _report(failures, skipped_full=True)
        return 1

    with tempfile.TemporaryDirectory(prefix="neo-extract-") as tmp:
        # 复制 crate 目录（只带源码与清单，跳过 target/ 之类的构建残留）
        for c in EXTRACT_SET:
            src = os.path.join(CRATES, c)
            if not os.path.isdir(src):
                continue
            shutil.copytree(
                src,
                os.path.join(tmp, c),
                ignore=shutil.ignore_patterns("target", "*.rs.bk"),
            )

        members = ", ".join(toml_str(c) for c in EXTRACT_SET if os.path.isdir(os.path.join(tmp, c)))
        root = f"""[workspace]
resolver = "2"
members = [{members}]

[workspace.package]
{os.linesep.join(pkg_lines)}

[workspace.dependencies]
{os.linesep.join(dep_lines)}
"""
        with open(os.path.join(tmp, "Cargo.toml"), "w") as f:
            f.write(root)

        print("  合成的新仓根（这部分就是新仓作者要自己写的东西）：")
        print(f"    成员：{len(members.split(',')) if members else 0} 个")
        print(f"    继承元数据键：{', '.join(sorted(pkg_keys_needed)) or '-'}")
        print(f"    继承依赖键：{', '.join(sorted(dep_keys_needed)) or '-'}")
        print("-" * 62)

        # `--no-deps`：只解析 manifest 与 workspace 继承，不解析整个依赖图。
        # 实测 0.1 秒，且对 E2 那类（路径依赖逃出）会直接报错。
        r = subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1"],
            cwd=tmp,
            capture_output=True,
            text=True,
            # 离线：这个守卫不该因为网络状况而红/绿（那是另一类问题）
            env={**os.environ, "CARGO_NET_OFFLINE": "true"},
        )
        if r.returncode != 0:
            failures.append("E6 cargo 无法解析合成的新仓：\n" + _indent(r.stderr.strip(), 7))
            _report(failures, skipped_full=True)
            return 1

        print("  [PASS] E1  可开源集内的 crate 都存在")
        print("  [PASS] E2  没有路径依赖逃出提取集")
        print("  [PASS] E3/E4  新仓根所需的继承键都能从主仓复制出来")
        print("  [PASS] E6  cargo 能解析合成的新仓（workspace 继承自洽）")

        if not full:
            print("-" * 62)
            print("  [SKIP] E7  完整编译演练（`--full` 才跑：gpui-kit 要编 ~1.5 分钟）")
            print("         发布前 / CI 请跑一次：python3 checks/check_extractable.py --full")
            print("-" * 62)
            print(f"✅ 可提取性守卫通过：{len(EXTRACT_SET)} 个 crate 的结构自洽，可整目录搬走")
            return 0

        # ── 4. 完整编译 + 测试（方案 DoD 里的"实际演练一次"）──────────────
        print("  正在做完整编译演练（会跑一会儿：要编 gpui-kit）…")
        target_dir = os.environ.get(
            "NEO_EXTRACT_TARGET_DIR", os.path.join(ROOT, "target", "extract-drill")
        )
        env = {**os.environ, "CARGO_TARGET_DIR": target_dir}
        for step, cmd in (
            ("E7  编译（cargo check --workspace）", ["cargo", "check", "--workspace"]),
            ("E8  测试（cargo test --workspace）", ["cargo", "test", "--workspace"]),
        ):
            r = subprocess.run(cmd, cwd=tmp, capture_output=True, text=True, env=env)
            if r.returncode != 0:
                failures.append(f"{step} 失败：\n" + _indent(r.stderr.strip()[-2000:], 7))
                _report(failures, skipped_full=False)
                return 1
            # 测试通过数写进输出：只报"绿"而不报规模，无法判断这是
            # 真的跑了测试还是"零个测试也算通过"
            detail = _last_test_result(r.stdout)
            print(f"  [PASS] {step}{detail}")

    _report(failures, skipped_full=False)
    return 0 if not failures else 1


def _dep_to_toml(spec):
    """把主仓根里的依赖声明转写成新仓根的写法。"""
    if isinstance(spec, str):
        return toml_str(spec)
    items = []
    for k, v in spec.items():
        if isinstance(v, str):
            items.append(f"{k} = {toml_str(v)}")
        elif isinstance(v, bool):
            items.append(f"{k} = {'true' if v else 'false'}")
        elif isinstance(v, list):
            inner = ", ".join(toml_str(x) for x in v)
            items.append(f"{k} = [{inner}]")
    return "{ " + ", ".join(items) + " }" if items else toml_str("*")


def _last_test_result(stdout):
    """从 cargo test 输出里取通过数（没有就返回空串）。"""
    passed = re.findall(r"test result: ok\. (\d+) passed", stdout)
    if not passed:
        return ""
    return f"（合计 {sum(int(x) for x in passed)} 个测试通过）"


def _indent(text, n):
    pad = " " * n
    return "\n".join(pad + line for line in text.splitlines())


def _report(failures, skipped_full):
    print("=" * 62)
    if failures:
        print(f"❌ 可提取性守卫失败，共 {len(failures)} 项：")
        for f in failures:
            print("   -", f)
        print()
        print("   这条守卫失败意味着：**把可开源集复制到空仓后编不过**。")
        print("   主仓里一切正常，所以问题只在真正搬走时才会暴露。")
    else:
        extra = "（结构检查；完整编译见 --full）" if skipped_full else ""
        print(f"✅ 可提取性守卫通过：可开源集复制到空仓能编译并跑测试{extra}")


if __name__ == "__main__":
    sys.exit(main())
