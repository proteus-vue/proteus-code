#!/usr/bin/env bash
# 选出**本仓钉版的那把 cargo** —— 被其它脚本 source 使用，不单独执行。
#
#   . "$(dirname "$0")/lib-rust-toolchain.sh"
#   ensure_pinned_cargo   # 失败即退出（exit 1），并把修法打在 stderr
#
# # 为什么需要它（这是实测踩到的真问题，不是防御性编程）
#
# 开发机上 `PATH` 常常先命中 Homebrew 的 cargo（`/opt/homebrew/bin` 排在
# `~/.cargo/bin` 前面）。而本仓 `rust-toolchain.toml` 钉的是 1.95 ——
# Cargo.lock 里的依赖树含 `hashbrown 0.17.1` / `dlopen2_derive 0.4.3` 这类
# 要求 `edition2024`（即 cargo ≥ 1.85）的包，**旧 cargo 连 manifest 都解析
# 不了**，报出来的是：
#
#     failed to parse manifest ... feature `edition2024` is required
#
# **它看起来像编译错误，实际是工具链选错**。症状离病因很远，所以每个会
# 调 cargo 的脚本都得先选对工具链，而不是各自在报错时困惑。
#
# # 修法（与 scripts/verify.sh 的预检同一套判定）
#
# 1. 若 `$HOME/.cargo/bin/cargo` 存在且 PATH 上先命中的不是它 → **优先用它**。
#    它是 rustup shim，会自己按 `rust-toolchain.toml` 选版本，用户无需改环境。
# 2. 核对版本与本仓钉版一致；不一致就明确失败并给修法（不要让它伪装成
#    编译错误散落到下游）。
#
# # 诚实边界
#
# - 钉版通道若不是具体版本号（如 `stable`），跳过版本核对（无法比对）。
# - 它只保证"选中的 cargo 与钉版一致"，不负责安装工具链。

set -uo pipefail

# 仓库根（本文件在 scripts/ 下）。
_rt_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# 从 rust-toolchain.toml **读**钉版号 —— 不在这里抄一遍（抄了会漂移）。
_rt_pinned() {
  local f="$_rt_root/rust-toolchain.toml"
  [ -f "$f" ] || return 0
  sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$f" | head -1
}

ensure_pinned_cargo() {
  # 1) 优先用 rustup shim（它会按 rust-toolchain.toml 自动选版本）。
  #
  # ⚠️ 判据是"**先命中的是不是它**"，不是"它在不在 PATH 里" —— 实测踩到：
  # `.cargo/bin` 往往已经在 PATH 里，但排在 Homebrew 后面，于是仍然选中
  # 旧 cargo。写成"不在 PATH 才加"会让这个函数**看起来做了事、实际没做**。
  if [ -x "$HOME/.cargo/bin/cargo" ] &&
     [ "$(command -v cargo 2>/dev/null)" != "$HOME/.cargo/bin/cargo" ]; then
    PATH="$HOME/.cargo/bin:$PATH"; export PATH
  fi

  if ! command -v cargo >/dev/null 2>&1; then
    echo "❌ 找不到 cargo。装 rustup 后重试：https://rustup.rs" >&2
    return 1
  fi

  local pinned have
  pinned="$(_rt_pinned)"
  # 只认 `cargo X.Y.Z` 那一行：首次用某 toolchain 时 rustup 会先打印
  # 若干行 "info: syncing channel updates..."，取首行会拿到 "syncing"。
  have="$(cargo --version 2>/dev/null | grep -m1 '^cargo ' | awk '{print $2}')"

  case "$pinned" in
    [0-9]*)
      case "$have" in
        "$pinned"*) return 0 ;;
        *)
          echo "❌ cargo 版本不匹配：当前 ${have:-未知}，本仓钉的是 ${pinned}（见 rust-toolchain.toml）" >&2
          echo "   —— 这是**工具链选错**，不是代码问题。下游若报「feature edition2024 is required」，" >&2
          echo "      看着像编译错误，实为同一根因。" >&2
          echo "   修法：让 PATH 先命中 rustup 的 \$HOME/.cargo/bin；未装 rustup 则先装它。" >&2
          return 1
          ;;
      esac
      ;;
    *)
      # 钉版不是具体版本号（或没读到）—— 无法核对，放行
      return 0
      ;;
  esac
}
