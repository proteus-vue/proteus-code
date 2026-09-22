#!/usr/bin/env bash
# 重生成 app-server 协议契约产物（JSON Schema + TypeScript + 方法表）。
#
# 什么时候跑：改了 `neo-protocol` 的 Op/EventMsg/嵌套类型，或
# `neo-host-appserver` 的方法参数/信封之后。产物入库，门禁会逐字节比对。
#
# 用法：bash scripts/gen_protocol_schema.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$ROOT/scripts/lib-rust-toolchain.sh"
ensure_pinned_cargo

OUT="$ROOT/crates/neo-host-appserver/schema"
export NEO_SCHEMA_OUT="$OUT"
cargo test -p neo-host-appserver --features schema export_schema -- --ignored --nocapture
echo "✅ 已写入 $OUT"
echo "   记得 git add 并提交 —— 否则 check_protocol_schema.py 会红。"
