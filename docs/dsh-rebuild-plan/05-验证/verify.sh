#!/usr/bin/env bash
# NEO 落地验证总入口
# 用法: bash 05-验证/verify.sh [--with-cargo]
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
export NEO_ROOT="$ROOT/06-原型代码"

cd "$HERE" || exit 1
PY=python3
command -v "$PY" >/dev/null 2>&1 || PY=python

declare -a CHECKS=(
  "check_architecture.py|架构守卫：依赖方向 / 无环 / 宿主隔离 / 协议层纯净"
  "check_protocol.py|协议确定性：Op 序列 -> EventMsg 序列 回放"
  "check_session_schema.py|会话 JSONL：append-only / 可重建 / schema"
  "check_config_layers.py|配置层级与模式解析：四级合并 / 敏感键拦截"
  "check_mode_matrix.py|沙箱 × 审批 正交双轴矩阵自洽"
)

echo
echo "############################################################"
echo "#  NEO 落地验证套件"
echo "#  验证对象: $ROOT"
echo "############################################################"

fail=0
for entry in "${CHECKS[@]}"; do
  script="${entry%%|*}"
  desc="${entry##*|}"
  echo
  echo "############################################################"
  echo "#  $desc"
  echo "############################################################"
  if "$PY" "checks/$script"; then
    :
  else
    fail=$((fail+1))
  fi
done

# ---- 可选：Rust 编译校验（需本地工具链，沙盒默认无）----
echo
echo "############################################################"
echo "#  Rust 原型编译校验（可选）"
echo "############################################################"
if command -v cargo >/dev/null 2>&1; then
  ( cd "$NEO_ROOT" && cargo check --workspace ) || fail=$((fail+1))
else
  echo "  [SKIP] 未检测到 cargo。本次交付的 Rust 原型未经编译验证。"
  echo "         请在本地执行：cd 06-原型代码 && cargo check --workspace"
  echo "         （沙盒环境无 Rust 工具链且无法联网安装，属已知限制 R11）"
fi

echo
echo "============================================================"
if [ "$fail" -eq 0 ]; then
  echo "✅ 全部验证通过"
  exit 0
else
  echo "❌ 有 $fail 组校验未通过"
  exit 1
fi
