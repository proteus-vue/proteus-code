#!/usr/bin/env bash
#
# 把 `neo` 二进制包成 macOS 的 .app —— 这是**键盘输入能被自动化投递**的前提。
#
#   bash scripts/make-app.sh                       # debug 构建，产物在 dist/NEO.app
#   bash scripts/make-app.sh --release             # release 构建
#   bash scripts/make-app.sh --features gpui       # 带上指定 cargo feature
#
# 为什么需要它（实测结论，不是推测）：
#
#   直接跑 `./target/debug/neo desktop --gpui` 时，进程的 `bundle_id` 是 **null**。
#   WindowServer 因此拿不到一个稳定的应用身份，于是：
#     - 合成长按/点击被拒："no stable WindowServer app/window identity"
#     - 键盘事件投不进去："app_ref did not resolve to a unique live application"
#   结果是 GUI 的键盘路径（打字、回车提交）**完全无法自动化验证** ——
#   只能靠人眼点、人眼敲，而"人眼敲过"留不下任何可复现的证据。
#
#   包成 .app 并赋予 `CFBundleIdentifier` 之后，键盘事件可以正常投递：
#   `type` 打到输入框、`key return` 真的触发提交，会话日志里能看到
#   `user_submitted`。这把"回车提交"从"读源码推断"变成了有机器证据的结论。
#
# 诚实边界：
#   - 这是**最小**包：Info.plist 只有必需键，无图标、未做签名/公证。
#     正式分发仍需要证书签名（见 PROJECT_MEMORY 的缺口清单）。
#   - 只处理 macOS。Linux/Windows 的桌面打包是另一件事，本脚本不假装覆盖。
#   - 用 `codesign --sign -` 做 ad-hoc 签名：仅为本机运行，不构成可信分发。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# 选对工具链：PATH 上若是 Homebrew 的旧 cargo，会报 "feature edition2024 is
# required" —— 看着像编译错误，实为工具链选错（详见该库文件说明）。
. "$ROOT/scripts/lib-rust-toolchain.sh"
ensure_pinned_cargo || exit 1

PROFILE="debug"
FEATURES=""
APP_DIR="${NEO_APP_DIR:-dist}"
APP_NAME="NEO"
BUNDLE_ID="${NEO_BUNDLE_ID:-dev.neokernel.neo}"

while [ $# -gt 0 ]; do
  case "$1" in
    --release) PROFILE="release"; shift ;;
    --features) FEATURES="${2:?--features 需要一个值}"; shift 2 ;;
    --features=*) FEATURES="${1#--features=}"; shift ;;
    --profile) PROFILE="${2:?--profile 需要一个值}"; shift 2 ;;
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "未知参数：$1（用 --help 看用法）" >&2; exit 2 ;;
  esac
done

[ "$(uname -s)" = "Darwin" ] || {
  echo "本脚本只处理 macOS 的 .app（当前系统：$(uname -s)）" >&2
  exit 1
}

# ── 1. 构建（带 feature 时 cargo 会自动重编，不必先清理）─────────────
say() { printf '%s\n' "$*" >&2; }
say "构建（profile=$PROFILE${FEATURES:+ features=$FEATURES}）…"
if [ "$PROFILE" = "release" ]; then
  # shellcheck disable=SC2086  # FEATURES 需要按空格拆成多个参数
  cargo build --release ${FEATURES:+--features "$FEATURES"}
else
  # shellcheck disable=SC2086
  cargo build ${FEATURES:+--features "$FEATURES"}
fi

BIN="target/$PROFILE/neo"
[ -x "$BIN" ] || { echo "构建产物不存在：$BIN" >&2; exit 1; }

# ── 2. 组装 .app ─────────────────────────────────────────────────────
APP="$APP_DIR/$APP_NAME.app"
say "打包 → $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/neo"

VERSION="$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$APP_NAME</string>
  <key>CFBundleDisplayName</key><string>$APP_NAME</string>
  <!-- bundle id 是键盘事件能被投递的关键（见文件头说明），不要删 -->
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleExecutable</key><string>neo</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION:-0.0.0}</string>
  <key>CFBundleVersion</key><string>1</string>
  <!-- 高 DPI：不声明这个，Retina 上会以 1x 渲染后放大（模糊） -->
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# ── 3. ad-hoc 签名 ───────────────────────────────────────────────────
# 未签名的包在部分环境下会被 Gatekeeper 直接拦下；ad-hoc 只为本机可运行。
if command -v codesign >/dev/null 2>&1; then
  codesign --force --deep --sign - "$APP" >/dev/null 2>&1 && say "已 ad-hoc 签名（仅供本机运行）"
else
  say "未找到 codesign —— 跳过签名（包应当仍可运行）"
fi

say ""
say "完成：$APP"
say "运行："
say "  open -a \"$ROOT/$APP\" --args desktop --gpui --provider mock"
