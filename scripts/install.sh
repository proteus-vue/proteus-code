#!/usr/bin/env bash
#
# neo 一键安装脚本 —— 下载预编译二进制，无需 Rust 工具链。
#
#   curl -fsSL https://raw.githubusercontent.com/proteus-vue/proteus-code/main/scripts/install.sh | sh
#   # efficiency-audit: ignore R002  （上面是文档示例，非脚本内真实调用）
#
# 环境变量：
#   NEO_VERSION      指定版本 tag（默认 latest），如 v0.1.0
#   NEO_INSTALL_DIR  安装目录（默认 ~/.local/bin）
#
# 诚实边界：预编译产物只覆盖下方 SUPPORTED 列出的平台。其它平台（如
# Linux aarch64、Windows、musl）请用 `cargo install --git`，见 README。
set -euo pipefail

REPO="proteus-vue/proteus-code"
BIN="neo"
INSTALL_DIR="${NEO_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${NEO_VERSION:-latest}"

say() { printf '%s\n' "$*" >&2; }
die() { say "❌ $*"; exit 1; }

# ── 探测平台 → 目标三元组 ─────────────────────────────────────────────
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Darwin) os_tag="apple-darwin" ;;
  Linux)  os_tag="unknown-linux-gnu" ;;
  *) die "不支持的系统：${os}（预编译仅覆盖 macOS / Linux；其它平台请用 cargo install --git，见 README）" ;;
esac
case "$arch" in
  arm64|aarch64) arch_tag="aarch64" ;;
  x86_64|amd64)  arch_tag="x86_64" ;;
  *) die "不支持的架构：$arch" ;;
esac
TARGET="${arch_tag}-${os_tag}"

# Linux 只发布了 x86_64（见 .github/workflows/release.yml 的产物矩阵）
if [ "$os_tag" = "unknown-linux-gnu" ] && [ "$arch_tag" != "x86_64" ]; then
  die "Linux $arch 暂无预编译产物。请改用：cargo install --git https://github.com/$REPO -p neo-code-cli --locked"
fi

# ── 解析下载地址 ──────────────────────────────────────────────────────
resolved="$VERSION"
if [ "$VERSION" = "latest" ]; then
  tag="$(curl -fsSL --connect-timeout 5 --max-time 30 "https://api.github.com/repos/$REPO/releases/latest" \
        | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
  [ -n "$tag" ] || die "无法从 GitHub API 获取最新版本（网络或限流）。可显式指定：NEO_VERSION=v0.1.0"
  resolved="$tag"
  base="https://github.com/$REPO/releases/download/$tag"
else
  base="https://github.com/$REPO/releases/download/$VERSION"
fi
asset="$BIN-$resolved-$TARGET.tar.gz"
dirname="$BIN-$resolved-$TARGET"

say "→ 平台 $TARGET"
say "→ 版本 $resolved"
say "→ 下载 $base/$asset"

# ── 下载 + 校验 + 安装（临时目录，用完即清）───────────────────────────
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

curl -fsSL --connect-timeout 5 --max-time 30 "$base/$asset" -o "$tmp/$asset" || die "下载失败：$base/$asset"

# 校验和（发布流程生成 .sha256）。缺失时如实告知，不静默跳过。
# macOS 自带 shasum（perl），Linux 一般只有 sha256sum —— 两者都探测。
sha_tool=""
command -v shasum >/dev/null 2>&1 && sha_tool="shasum -a 256"
[ -z "$sha_tool" ] && command -v sha256sum >/dev/null 2>&1 && sha_tool="sha256sum"
if curl -fsSL --connect-timeout 5 --max-time 30 "$base/$asset.sha256" -o "$tmp/$asset.sha256" 2>/dev/null; then
  [ -n "$sha_tool" ] || die "提供了校验和但本机既无 shasum 也无 sha256sum，无法验证"
  ( cd "$tmp" && $sha_tool -c "$asset.sha256" >/dev/null 2>&1 ) \
    || die "校验和不匹配 —— 下载已损坏或被篡改，已中止"
  say "✓ 校验和通过"
else
  say "⚠️  该 Release 未提供校验和文件，跳过验证"
fi

tar -xzf "$tmp/$asset" -C "$tmp"
mkdir -p "$INSTALL_DIR"
# 先删后拷：macOS 上直接 cp 覆盖已签名二进制会让签名失效（SIGKILL）
rm -f "$INSTALL_DIR/$BIN"
cp "$tmp/$dirname/$BIN" "$INSTALL_DIR/$BIN"
chmod +x "$INSTALL_DIR/$BIN"

say "✓ 已安装到 $INSTALL_DIR/$BIN"

# ── PATH 提示 ─────────────────────────────────────────────────────────
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) say ""
     say "⚠️  $INSTALL_DIR 不在 PATH 里。把它加进去："
     say "     export PATH=\"$INSTALL_DIR:\$PATH\"   # 写进 ~/.zshrc 或 ~/.bashrc 以持久化" ;;
esac

say ""
say "快速开始（先装到 PATH 再运行）："
say "  neo tui --provider mock     # 离线全览界面与交互，无需 API key"
say "  neo --help                  # 查看全部用法"
