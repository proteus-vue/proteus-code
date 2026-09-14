#!/usr/bin/env bash
#
# 把 GitHub Release 的预编译二进制打成 npm 包并发布。
#
# 产物是两个层次：
#   1. 平台包 neo-code-<os>-<arch> —— 内含该平台的 neo 二进制（os/cpu 字段
#      让 npm 只装匹配项）
#   2. 主包 neo-code —— 只有 bin/neo.js，通过 optionalDependencies 拽平台包
#
# 为什么这样拆（而不是单个包 + postinstall 下载）：安装期不执行脚本、
# 不联网。postinstall 里下载可执行文件是供应链注入的典型入口，本项目
# 在安全上刻意避开 —— 与 §0 里"可执行文件就是供应链注入"是同一条判据。
#
# 用法：
#   bash scripts/publish-npm.sh --dist <目录> --version <x.y.z> --dry-run
#   bash scripts/publish-npm.sh --dist <目录> --version <x.y.z>
#
# --dist 里应有 release.yml 产出的 neo-<tag>-<target>.tar.gz。
# --dry-run 只 npm pack + 本地装载冒烟，不发布。
#
# 已发布的版本会被跳过（幂等，可安全重跑）。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST=""
VERSION=""
DRY=0

while [ $# -gt 0 ]; do
  case "$1" in
    --dist) DIST="${2:?--dist 需要目录}"; shift 2 ;;
    --version) VERSION="${2:?--version 需要版本}"; shift 2 ;;
    --dry-run) DRY=1; shift ;;
    -h|--help) sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "未知参数：$1" >&2; exit 2 ;;
  esac
done

say() { printf '%s\n' "$*" >&2; }
die() { say "❌ $*"; exit 1; }

[ -n "$DIST" ] || die "缺少 --dist"
[ -d "$DIST" ] || die "--dist 目录不存在：$DIST"
[ -n "$VERSION" ] || die "缺少 --version"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]] \
  || die "版本不是合法 semver：$VERSION"

# 版本一致性（防漂移）：npm 包版本来自 git tag，而二进制里 `neo --version`
# 报的版本来自 Cargo.toml。两者不一致会产出"包是 0.1.0、程序自称 0.2.0"
# 这种极难察觉的错配 —— 在发布出口处硬校验，别等用户发现。
CARGO_VERSION="$(python3 - "$ROOT/Cargo.toml" <<'PY'
import re, sys
for line in open(sys.argv[1], encoding="utf-8"):
    m = re.match(r'\s*version\s*=\s*"([^"]+)"', line)
    if m:
        print(m.group(1)); break
PY
)"
[ -n "$CARGO_VERSION" ] || die "无法从 Cargo.toml 读出版本"
[ "$CARGO_VERSION" = "$VERSION" ] || die \
"版本漂移：Cargo.toml 的 [workspace.package] version = \"${CARGO_VERSION}\"，但本次发布版本是 ${VERSION}。
 二者必须相同 —— 二进制自报版本取自 Cargo.toml，npm 包版本取自 git tag。
 修法：把 Cargo.toml 的版本改成 ${VERSION} 再重打 tag；或让 tag（去掉 v 前缀）等于 ${CARGO_VERSION}。"

command -v npm >/dev/null 2>&1 || die "未找到 npm"
[ -f "$ROOT/npm/neo-code/package.json" ] || die "未找到 npm/neo-code"

# rust target : npm 平台包名 : os : cpu
TARGETS=(
  "aarch64-apple-darwin:neo-code-darwin-arm64:darwin:arm64"
  "x86_64-apple-darwin:neo-code-darwin-x64:darwin:x64"
  "x86_64-unknown-linux-gnu:neo-code-linux-x64:linux:x64"
)

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

already_published() { # $1=name  —— 已发布该版本则跳过（幂等）
  local code
  code="$(curl -fsS -o /dev/null -w '%{http_code}' --connect-timeout 5 --max-time 20 \
          "https://registry.npmjs.org/$(printf '%s' "$1" | sed 's|/|%2f|')/$VERSION" 2>/dev/null || true)"
  [ "$code" = "200" ]
}

publish_or_pack() { # $1=包目录
  if [ "$DRY" -eq 1 ]; then
    npm pack --pack-destination "$WORK/packed" --silent "$1" >/dev/null
    return 0
  fi
  local flags=()
  # CI 里带 provenance（签名证明 tarball 出自这次构建）——公开仓库才有意义
  [ "${GITHUB_ACTIONS:-}" = "true" ] && flags+=(--provenance)
  npm publish "${flags[@]}" "$1"
}

mkdir -p "$WORK/packed"
platform_names=()

# ── 1. 平台包 ──────────────────────────────────────────────────────────
say "== 平台包（版本 ${VERSION}）=="
for entry in "${TARGETS[@]}"; do
  IFS=':' read -r target pkg os cpu <<< "$entry"
  tarball="$(ls "$DIST"/neo-*-"$target".tar.gz 2>/dev/null | head -1 || true)"
  [ -n "$tarball" ] || { say "⏭  ${pkg}：${DIST} 里没有 $target 的产物，跳过"; continue; }

  dir="$WORK/pkg-$pkg"
  mkdir -p "$dir/bin"
  tar -xzf "$tarball" -C "$WORK"
  src="$(ls "$WORK"/neo-*/neo 2>/dev/null | head -1 || true)"
  [ -n "$src" ] || die "$tarball 里找不到 neo 二进制"
  # 只取符合本平台的那份（同名 tar 可能有多个）
  cp "$src" "$dir/bin/neo"
  chmod +x "$dir/bin/neo"
  # 清掉解出来的兄弟目录，避免下一个 target 认错
  find "$WORK" -maxdepth 1 -type d -name 'neo-*' -exec rm -rf {} + 2>/dev/null || true

  python3 - "$dir/package.json" "$pkg" "$VERSION" "$os" "$cpu" <<'PY'
import json, sys
path, name, version, os_, cpu = sys.argv[1:6]
json.dump({
    "name": name,
    "version": version,
    "description": f"neo 的预编译二进制（{os_} {cpu}）—— 一般由 neo-code 自动选中，无需手动安装",
    "license": "MIT",
    "os": [os_],
    "cpu": [cpu],
    "files": ["bin/neo"],
    "repository": {"type": "git", "url": "git+https://github.com/proteus-vue/proteus-code.git"},
    "homepage": "https://github.com/proteus-vue/proteus-code",
    # pnpm/yarn berry：二进制包需要"不虚拟化"才能正常执行
    "preferUnplugged": True,
}, open(path, "w"), indent=2, ensure_ascii=False)
open(path, "a").write("\n")
PY

  if already_published "$pkg"; then
    say "⏭  $pkg@$VERSION 已发布，跳过"
  else
    say "→ ${pkg}（$(du -h "$dir/bin/neo" | cut -f1)）"
    publish_or_pack "$dir"
  fi
  platform_names+=("$pkg")
done

[ "${#platform_names[@]}" -gt 0 ] || die "没有任何平台产物 —— 检查 $DIST 里的 tar 命名"

# ── 2. 主包（版本 + optionalDependencies 对齐）────────────────────────
say ""
say "== 主包 neo-code =="
main="$WORK/pkg-neo-code"
mkdir -p "$main"
cp -R "$ROOT/npm/neo-code/." "$main/"
python3 - "$main/package.json" "$VERSION" "${platform_names[@]}" <<'PY'
import json, sys
path, version, *platforms = sys.argv[1:]
pkg = json.load(open(path))
pkg["version"] = version
# 只声明**本次确实发布了**的平台包，避免指向不存在的版本
pkg["optionalDependencies"] = {name: version for name in sorted(platforms)}
json.dump(pkg, open(path, "w"), indent=2, ensure_ascii=False)
open(path, "a").write("\n")
print("  optionalDependencies:", ", ".join(sorted(platforms)), file=sys.stderr)
PY

if already_published "neo-code"; then
  say "⏭  neo-code@$VERSION 已发布，跳过"
else
  publish_or_pack "$main"
fi

# ── 3. dry-run：本地装载冒烟（证明 wrapper 真能把二进制跑起来）────────
if [ "$DRY" -eq 1 ]; then
  say ""
  say "== 本地装载冒烟（不发布）=="
  host_key="$(node -p 'process.platform + "-" + process.arch')"
  say "主机平台：$host_key"
  smoke="$WORK/smoke"
  mkdir -p "$smoke/node_modules"
  cp -R "$main" "$smoke/node_modules/neo-code"
  # 把**本机对应**的平台包放进 node_modules（模拟 npm 装好后的布局）
  for pkg in "${platform_names[@]}"; do
    cp -R "$WORK/pkg-$pkg" "$smoke/node_modules/$pkg"
  done
  if node "$smoke/node_modules/neo-code/bin/neo.js" --version; then
    say "✅ wrapper 冒烟通过（--version 由真二进制输出）"
  else
    die "wrapper 冒烟失败 —— 本机平台包不在产物里？看上面 host 平台是否匹配"
  fi
  say ""
  say "npm pack 产物："
  ls -1 "$WORK/packed" | sed 's/^/  /'
fi

say ""
if [ "$DRY" -eq 1 ]; then
  say "✅ 演练完成（未发布）。去掉 --dry-run 即真发布。"
else
  say "✅ 已发布：${platform_names[*]} + neo-code（版本 ${VERSION}）"
  say "   用户安装：npm install -g neo-code"
fi
