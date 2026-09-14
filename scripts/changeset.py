#!/usr/bin/env python3
"""changeset —— 轻量、零依赖的变更碎片工作流。

**为什么不用 @changesets/cli**：它只认 npm 工作区的 package.json，管不了
Cargo.toml，而本项目的版本真源恰恰是 Cargo 的 `[workspace.package] version`。
用它等于让工具去管**非权威**的那份版本号，再手写胶水同步权威的这份 ——
本末倒置，还会往纯 Rust 仓库塞一套 Node 工具链（与项目一贯的零依赖立场相悖）。
changeset 的精髓是「每个改动写一条碎片、由机器聚合」，那部分自己实现即可。

**本仓库是单一版本工作区**：23 个 crate 共享 `[workspace.package] version`，
所以碎片只声明 bump 级别，不写包名。

碎片格式（`.changeset/<名字>.md`）：

    ---
    bump: minor
    ---

    一句话说明这个改动（会原样进 CHANGELOG）。

子命令：

    new --bump <patch|minor|major> --note "..."   新建一条碎片
    status [--machine]                            列出待发碎片
    version                                       汇总 → 改 Cargo.toml / Cargo.lock
                                                  / CHANGELOG.md → 删除已消费碎片
    check --base <ref>                            校验（CI 门禁用）：改动是否带了碎片

约定：**stdout 只输出机器可读的 KEY=VALUE**，人类可读的说明走 stderr。
这样工作流能直接消费输出，不必解析散文。
"""

from __future__ import annotations

import argparse
import datetime as _dt
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CHANGESET_DIR = ROOT / ".changeset"
CARGO_TOML = ROOT / "Cargo.toml"
CARGO_LOCK = ROOT / "Cargo.lock"
CHANGELOG = ROOT / "CHANGELOG.md"

# .changeset/ 下非碎片文件（README 与配置）
NON_FRAGMENT = {"README.md", "config.json"}

BUMPS = ["patch", "minor", "major"]
LEVEL_ORDER = {b: i for i, b in enumerate(BUMPS)}

# 需要 changeset 的路径（产品面）。scripts/ 与 .github/ 属基础设施，
# 改它们不改变用户拿到的东西，故不强制。
PRODUCT_PREFIXES = ("crates/", "npm/")


def say(msg: str = "") -> None:
    """人类可读输出 → stderr（stdout 留给机器消费）。"""
    print(msg, file=sys.stderr)


# ── 碎片读写 ────────────────────────────────────────────────────────────

def fragment_paths() -> list[Path]:
    if not CHANGESET_DIR.is_dir():
        return []
    return sorted(
        p
        for p in CHANGESET_DIR.glob("*.md")
        if p.name not in NON_FRAGMENT and not p.name.startswith(".")
    )


def parse_fragment(path: Path) -> tuple[str, str]:
    """返回 (bump, body)。bump 取所有声明里的最高级。

    支持两种前区写法：
      bump: minor                 ← 本仓库的规范写法（单一版本）
      "neo-code-cli": minor       ← changesets 习惯写法；名字被忽略，只取级别
    """
    text = path.read_text(encoding="utf-8")
    lines = text.splitlines()

    # 找前区：首个非空行必须是 ---，直到下一个 ---
    i = 0
    while i < len(lines) and not lines[i].strip():
        i += 1
    if i >= len(lines) or lines[i].strip() != "---":
        raise SystemExit(f"❌ {path} 缺少前区（文件需以 --- 开头）")
    start = i + 1
    j = start
    while j < len(lines) and lines[j].strip() != "---":
        j += 1
    if j >= len(lines):
        raise SystemExit(f"❌ {path} 前区没有结束的 ---")

    frontmatter = lines[start:j]
    body = "\n".join(lines[j + 1:]).strip()

    declared: list[str] = []
    for line in frontmatter:
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        m = re.match(r'^"?([\w@/.\-]+)"?\s*:\s*(\w+)\s*$', line)
        if not m:
            raise SystemExit(f"❌ {path} 前区无法解析：{line!r}")
        level = m.group(2).lower()
        if level not in LEVEL_ORDER:
            raise SystemExit(
                f"❌ {path} 的 bump 级别 {level!r} 无效（只能是 {'/'.join(BUMPS)}）"
            )
        declared.append(level)

    if not declared:
        raise SystemExit(f"❌ {path} 没有声明 bump 级别")
    if not body:
        raise SystemExit(f"❌ {path} 没有正文（正文会进 CHANGELOG）")

    return max(declared, key=lambda b: LEVEL_ORDER[b]), body


def collect() -> list[tuple[Path, str, str]]:
    """返回 [(path, bump, body)]，按文件名排序。"""
    out = []
    for p in fragment_paths():
        bump, body = parse_fragment(p)
        out.append((p, bump, body))
    return out


# ── 版本计算 ────────────────────────────────────────────────────────────

def read_version() -> str:
    in_section = False
    for line in CARGO_TOML.read_text(encoding="utf-8").splitlines():
        s = line.strip()
        if s.startswith("["):
            in_section = s == "[workspace.package]"
            continue
        if in_section:
            m = re.match(r'^version\s*=\s*"([^"]+)"\s*$', s)
            if m:
                return m.group(1)
    raise SystemExit("❌ 无法从 Cargo.toml 的 [workspace.package] 读出 version")


def write_version(new: str) -> None:
    """只改 [workspace.package] 里的 version —— 不能碰 rust-version、
    也不能碰 [workspace.dependencies] 里的依赖版本。"""
    lines = CARGO_TOML.read_text(encoding="utf-8").split("\n")
    in_section = False
    for idx, line in enumerate(lines):
        s = line.strip()
        if s.startswith("["):
            in_section = s == "[workspace.package]"
            continue
        if in_section and re.match(r'^version\s*=\s*"', s):
            lines[idx] = f'version = "{new}"'
            CARGO_TOML.write_text("\n".join(lines), encoding="utf-8")
            return
    raise SystemExit("❌ 没能在 [workspace.package] 里改写 version")


def bump_version(version: str, level: str) -> str:
    """语义化推进。预发布后缀（-rc.1）与构建元数据（+build）会被丢弃 ——
    发正式版时它们本就该消失；要出预发布版请手工改 Cargo.toml。"""
    core = version.split("+", 1)[0].split("-", 1)[0]
    try:
        major, minor, patch = (int(x) for x in core.split("."))
    except ValueError:
        raise SystemExit(f"❌ 当前版本不是三段式 semver：{version!r}")
    if level == "major":
        major, minor, patch = major + 1, 0, 0
    elif level == "minor":
        minor, patch = minor + 1, 0
    else:
        patch += 1
    return f"{major}.{minor}.{patch}"


def update_internal_dep_versions(old: str, new: str) -> int:
    """把各 crate 清单里**内部路径依赖**的版本要求从 old 改成 new。

    为什么必须有这一步：crate 之间写的是
        neo-protocol = { path = "../neo-protocol", version = "0.1.0" }
    而 semver 里 `^0.1.0` **不匹配** 0.2.0（0.x 有特殊规则）。只改工作区版本
    而不改这些要求，cargo 会直接失败：
        failed to select a version for the requirement `neo-protocol = "^0.1.0"`
    实测确认过（不是推测）。

    只改"同时含 path = 且版本恰好等于 old"的行 —— 这样外部依赖
    （serde = "1"、wry = "0.46.1"）绝不会被误伤。
    """
    touched = 0
    for manifest in sorted((ROOT / "crates").glob("*/Cargo.toml")):
        lines = manifest.read_text(encoding="utf-8").split("\n")
        changed = False
        for idx, line in enumerate(lines):
            if 'path = "' not in line:
                continue
            if re.search(rf'version\s*=\s*"{re.escape(old)}"', line):
                lines[idx] = re.sub(
                    rf'(version\s*=\s*"){re.escape(old)}(")',
                    rf"\g<1>{new}\g<2>",
                    line,
                )
                changed = True
        if changed:
            manifest.write_text("\n".join(lines), encoding="utf-8")
            touched += 1
    return touched


def update_npm_version(old: str, new: str) -> None:
    """同步 npm 主包清单的版本（含 optionalDependencies 里的平台包版本）。

    发布时 publish-npm.sh 会**按 tag 重写**这些字段，所以这里不是发布所必需；
    但让它与 Cargo 版本一致，可以避免"仓库里两个版本号对不上"的困惑。
    """
    pkg = ROOT / "npm" / "neo-code" / "package.json"
    if not pkg.exists():
        return
    data = json.loads(pkg.read_text(encoding="utf-8"))
    if data.get("version") == old:
        data["version"] = new
        optional = data.get("optionalDependencies") or {}
        data["optionalDependencies"] = {
            name: (new if ver == old else ver) for name, ver in optional.items()
        }
        pkg.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def refresh_lockfile() -> None:
    """改完 Cargo.toml 后刷新 Cargo.lock（23 个成员 + 内部依赖的版本号）。

    交给 cargo 做（而不是手改锁文件）：手改容易漏、也容易改错块。
    """
    for extra in (["--offline"], []):
        try:
            subprocess.run(
                ["cargo", "metadata", "--format-version", "1", *extra],
                cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                check=True,
            )
            return
        except FileNotFoundError:
            say("⚠️  未找到 cargo —— Cargo.lock 未刷新，提交前请手动跑一次 cargo metadata")
            return
        except subprocess.CalledProcessError:
            continue
    say("⚠️  cargo metadata 刷新 Cargo.lock 失败 —— 提交前请手动跑一次")


# ── CHANGELOG ───────────────────────────────────────────────────────────

HEADER = """# Changelog

本项目所有值得记录的变更都会写在这里。

格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，
版本遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

版本号与条目**由 `scripts/changeset.py version` 从 `.changeset/` 碎片生成**，
不要手改本文件里由它维护的段落。更早的历史见 git log 与 `PROJECT_MEMORY.md`。
"""

LEVEL_TITLE = {"major": "Major Changes", "minor": "Minor Changes", "patch": "Patch Changes"}


def update_changelog(version: str, entries: list[tuple[str, str]]) -> None:
    # 按级别分组，组内保持碎片文件顺序
    grouped: dict[str, list[str]] = {b: [] for b in reversed(BUMPS)}
    for bump, body in entries:
        grouped[bump].append(body)

    section = [f"## {version}", ""]
    for bump in reversed(BUMPS):
        if not grouped[bump]:
            continue
        section.append(f"### {LEVEL_TITLE[bump]}")
        section.append("")
        for body in grouped[bump]:
            lines = body.splitlines()
            section.append(f"- {lines[0]}")
            for cont in lines[1:]:
                section.append(f"  {cont}")
        section.append("")
    rendered = "\n".join(section).rstrip() + "\n"

    if not CHANGELOG.exists():
        CHANGELOG.write_text(f"{HEADER}\n{rendered}", encoding="utf-8")
        return

    text = CHANGELOG.read_text(encoding="utf-8")
    # 插到第一个既有 "## " 之前（即最新版本放最上面）
    lines = text.splitlines(keepends=True)
    insert_at = len(lines)
    for i, line in enumerate(lines):
        if line.startswith("## "):
            insert_at = i
            break
    head = "".join(lines[:insert_at]).rstrip() + "\n\n"
    tail = "".join(lines[insert_at:])
    CHANGELOG.write_text(f"{head}{rendered}\n{tail.lstrip()}", encoding="utf-8")


# ── 子命令 ──────────────────────────────────────────────────────────────

def cmd_new(args) -> int:
    level = args.bump.lower()
    if level not in LEVEL_ORDER:
        raise SystemExit(f"❌ --bump 只能是 {'/'.join(BUMPS)}")
    note = (args.note or "").strip()
    if not note:
        raise SystemExit("❌ 缺少 --note（一句话说明这个改动，会进 CHANGELOG）")

    CHANGESET_DIR.mkdir(exist_ok=True)
    stamp = _dt.datetime.now().strftime("%Y%m%d%H%M%S")
    slug = re.sub(r"[^a-z0-9]+", "-", note.lower()).strip("-")[:40] or "change"
    path = CHANGESET_DIR / f"{stamp}-{slug}.md"
    n = 2
    while path.exists():
        path = CHANGESET_DIR / f"{stamp}-{slug}-{n}.md"
        n += 1

    path.write_text(f"---\nbump: {level}\n---\n\n{note}\n", encoding="utf-8")
    say(f"✅ 已创建 {path.relative_to(ROOT)}")
    say(f"   级别 {level}；与改动一起提交进你的 PR。")
    print(f"FRAGMENT={path.name}")
    return 0


def cmd_status(args) -> int:
    frags = collect()
    machine = getattr(args, "machine", False)
    current = read_version()

    if machine:
        # CURRENT_VERSION 总是输出，PENDING 为 0 时不输出 MAX_BUMP/NEXT_VERSION
        print(f"CURRENT_VERSION={current}")
        print(f"PENDING={len(frags)}")
        if frags:
            level = max((b for _, b, _ in frags), key=lambda b: LEVEL_ORDER[b])
            print(f"MAX_BUMP={level}")
            print(f"NEXT_VERSION={bump_version(current, level)}")
        return 0

    if not frags:
        say(f"没有待发 changeset（当前版本 {current}）")
        return 0

    level = max((b for _, b, _ in frags), key=lambda b: LEVEL_ORDER[b])
    say(f"待发 changeset：{len(frags)} 条（最高级别 {level}）")
    for p, bump, body in frags:
        head = body.splitlines()[0]
        say(f"  [{bump:5}] {p.name} — {head}")
    say("")
    say(f"{current} → {bump_version(current, level)}（跑 version 应用）")
    return 0


def cmd_version(args) -> int:
    frags = collect()
    if not frags:
        say("没有待发 changeset —— 无事可做。")
        return 0

    old = read_version()
    level = max((b for _, b, _ in frags), key=lambda b: LEVEL_ORDER[b])
    new = bump_version(old, level)
    if args.dry_run:
        say(f"[dry-run] {old} → {new}（最高级别 {level}，{len(frags)} 条碎片）")
        for p, bump, body in frags:
            say(f"  [{bump:5}] {body.splitlines()[0]}")
        return 0

    write_version(new)
    # 内部依赖的版本要求必须一起改（见 update_internal_dep_versions 的说明），
    # 顺序要紧：先改要求、再刷新锁文件，否则 cargo 会在解析时报错。
    touched = update_internal_dep_versions(old, new)
    update_npm_version(old, new)
    refresh_lockfile()
    update_changelog(new, [(b, body) for _, b, body in frags])
    for p, _, _ in frags:
        p.unlink()

    say(f"✅ {old} → {new}（{len(frags)} 条碎片已并入 CHANGELOG 并删除）")
    say(f"   已改：Cargo.toml、{touched} 个 crate 清单的内部依赖版本、Cargo.lock、CHANGELOG.md")
    print(f"VERSION_OLD={old}")
    print(f"VERSION_NEW={new}")
    print(f"COUNT={len(frags)}")
    return 0


def cmd_check(args) -> int:
    """CI 门禁：产品面有改动却没带 changeset 时失败。"""
    if os.environ.get("CHANGESET_SKIP") == "1":
        say("设置了 CHANGESET_SKIP=1 —— 跳过检查")
        return 0

    base = args.base
    try:
        changed = subprocess.run(
            ["git", "diff", "--name-only", f"{base}...HEAD"],
            cwd=ROOT, capture_output=True, text=True, check=True,
        ).stdout.split()
    except subprocess.CalledProcessError as e:
        say(f"⚠️  无法对 {base} 取 diff（{e.stderr.strip() or '未知错误'}）—— 放行")
        return 0

    product = [f for f in changed if f.startswith(PRODUCT_PREFIXES)]
    added_fragments = [
        f for f in changed
        if f.startswith(".changeset/") and f.endswith(".md")
        and Path(f).name not in NON_FRAGMENT
    ]
    # 只认"新增"的碎片：删掉又加回来不算（用 diff-filter 再确认一次）
    try:
        added = subprocess.run(
            ["git", "diff", "--name-only", "--diff-filter=A", f"{base}...HEAD", "--", ".changeset"],
            cwd=ROOT, capture_output=True, text=True, check=True,
        ).stdout.split()
        added_fragments = [
            f for f in added
            if f.endswith(".md") and Path(f).name not in NON_FRAGMENT
        ]
    except subprocess.CalledProcessError:
        pass

    if added_fragments:
        say(f"✅ 带了 changeset：{', '.join(added_fragments)}")
        return 0

    if not product:
        say("没有产品面改动（crates/ 或 npm/）—— 无需 changeset，通过")
        return 0

    say("❌ 有产品面改动，但没有新增 changeset。")
    say("")
    say("   改动到的产品文件：")
    for f in product[:10]:
        say(f"     {f}")
    if len(product) > 10:
        say(f"     …还有 {len(product) - 10} 个")
    say("")
    say("   请加一条碎片：")
    say('     python3 scripts/changeset.py new --bump patch --note "一句话说明"')
    say("")
    say("   若本 PR 确实不该有 changeset（纯重构/CI/文档），给 PR 打上")
    say("   `no-changeset` 标签即可豁免。")
    return 1


def main() -> int:
    ap = argparse.ArgumentParser(prog="changeset", description="changeset 工作流（native 实现）")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p_new = sub.add_parser("new", help="新建一条碎片")
    p_new.add_argument("--bump", required=True, choices=BUMPS)
    p_new.add_argument("--note", required=True, help="一句话说明（会进 CHANGELOG）")
    p_new.set_defaults(func=cmd_new)

    p_st = sub.add_parser("status", help="列出待发碎片")
    p_st.add_argument("--machine", action="store_true", help="只输出 KEY=VALUE")
    p_st.set_defaults(func=cmd_status)

    p_v = sub.add_parser("version", help="汇总碎片 → 推进版本 → 写 CHANGELOG")
    p_v.add_argument("--dry-run", action="store_true")
    p_v.set_defaults(func=cmd_version)

    p_c = sub.add_parser("check", help="CI 门禁：产品面改动是否带了 changeset")
    p_c.add_argument("--base", required=True, help="对比基线 ref，如 origin/main")
    p_c.set_defaults(func=cmd_check)

    args = ap.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
