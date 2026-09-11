#!/usr/bin/env bash
# NEO 全套门禁入口。
#
# 三部分：
#   1. Rust 工程门禁：cargo test（含内核 conformance、内存有界性、SPI conformance）
#   2. 架构与协议守卫：docs/neo-plan/05-验证/ 的 Python 检查（依赖方向、协议确定性、
#      会话格式、配置层叠、模式矩阵、SPI 合规）
#   3. 工具链卫生：零 warning（warning 是未来错误的温床）
#
# 用法：bash scripts/verify.sh
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
PLAN_CHECKS="$ROOT/docs/neo-plan/05-验证"

if ! command -v cargo >/dev/null 2>&1; then
  [ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
fi

fail=0
hr() { printf '%s\n' "############################################################"; }

hr; echo "#  NEO 门禁"; echo "#  仓库: $ROOT"; hr

# ── 1. Rust 测试（内核 conformance 是主门禁）────────────────────────────
hr; echo "#  Rust: cargo test --workspace（内核 / 内存 / SPI conformance）"; hr
if command -v cargo >/dev/null 2>&1; then
  ( cd "$ROOT" && cargo test --workspace ) || fail=$((fail+1))
else
  echo "  [SKIP] 未找到 cargo —— Rust 门禁未执行。请安装 Rust 后重跑。"
  fail=$((fail+1))
fi

# ── 2. 警告即错误（工具链卫生）──────────────────────────────────────────
hr; echo "#  Rust: 零 warning 检查"; hr
if command -v cargo >/dev/null 2>&1; then
  warn_out="$(cd "$ROOT" && cargo check --workspace --all-targets 2>&1 | grep -c '^warning' || true)"
  if [ "${warn_out:-0}" -eq 0 ]; then
    echo "  ✅ 无 warning"
  else
    echo "  ❌ 有 $warn_out 条 warning —— warning 是未来错误的温床，请清零"
    fail=$((fail+1))
  fi
fi

# ── 3. 架构 / 协议 / 会话 / 配置 / 模式 / SPI 守卫 ───────────────────────
if [ -d "$PLAN_CHECKS" ]; then
  export NEO_ROOT="$ROOT"
  for entry in \
    "check_architecture.py|架构守卫：依赖方向 / 无环 / 宿主隔离 / 协议层纯净" \
    "check_protocol.py|协议确定性：Op 序列 -> EventMsg 序列" \
    "check_session_schema.py|会话日志：append-only / 可重建 / schema" \
    "check_config_layers.py|配置层级与模式解析" \
    "check_mode_matrix.py|沙箱 × 审批 正交双轴矩阵自洽" \
    "check_spi_conformance.py|SPI 合规：契约 / >=2 后端 / conformance" ; do
    script="${entry%%|*}"; desc="${entry##*|}"
    hr; echo "#  $desc"; hr
    py=python3; command -v "$py" >/dev/null 2>&1 || py=python
    ( cd "$PLAN_CHECKS" && "$py" "checks/$script" ) || fail=$((fail+1))
  done
else
  echo "  [SKIP] 未找到 $PLAN_CHECKS"
fi

# ── 4. 执行效率规范（ai-efficiency-rules）───────────────────────────────
#
# 与其它守卫同级：把「不要固定 sleep、不要重复拉取远程、不要无上限重试」
# 从**口头约定**变成**机器可判**。先前这些只写在 AGENTS.md 里靠自觉，
# 而本项目已经因为固定 sleep 浪费过一整轮时间。
hr; echo "#  执行效率：固定盲等 / 重复拉取 / 无退出轮询（ai-efficiency-rules）"; hr
AUDIT="$ROOT/docs/ai-efficiency-rules/scripts/audit_efficiency.py"
if [ -f "$AUDIT" ]; then
  py=python3; command -v "$py" >/dev/null 2>&1 || py=python
  # 只让 error 级卡门禁（warn 多为启发式，容易误报）；
  # 扫 crates/ scripts/ docs/ —— 排除 docs/ai-efficiency-rules 自身，
  # 它的规则表里含有用于**说明**的违规样例（会自我命中，是已知误报）。
  if ( cd "$ROOT" && "$py" "$AUDIT" --path crates --path scripts --fail-on error ) >/tmp/neo-eff.log 2>&1; then
    echo "  ✅ 未检测到固定盲等 / 重复拉取 / 无退出轮询"
  else
    echo "  ❌ 检测到执行效率违规（error 级）："
    sed 's/^/     /' /tmp/neo-eff.log | tail -20
    fail=$((fail+1))
  fi
else
  echo "  [SKIP] 未找到 $AUDIT（skill 未安装？见 docs/ai-efficiency-rules/）"
fi

echo
echo "============================================================"
if [ "$fail" -eq 0 ]; then echo "✅ 全部门禁通过"; exit 0
else echo "❌ 有 $fail 组未通过"; exit 1; fi
