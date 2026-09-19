#!/usr/bin/env bash
# 键盘可达验证 —— 真机走一遍 Tab 顺序（DoD 九宫格第 1 项）。
#
#   bash scripts/check-keyboard-reach.sh            # 走 6 步
#   bash scripts/check-keyboard-reach.sh --steps 3  # 指定步数
#
# # 为什么是脚本 + 真机，而不是单元测试
#
# 试过在无头测试里驱动 Tab 顺序，**四版都没成**（见 §4.64(bp) 的记录）。
# 根因是框架时序：tab stop 在 paint 时写进 `next_frame.tab_stops`，而
# `focus_next` 读 `rendered_frame.tab_stops`，两者在 `Window::draw` 末尾才交换。
# 用测试替身手写这套时序，等于**把被测对象的复杂度复制一份到测试里**。
#
# 所以改成：**在真窗口上让应用自己走**，把落点打出来。探针 `NEO_GUI_TAB=<n>`
# 就是在 produce 代码里调 `focus_next` n 次（与 `Root` 把 Tab 接到它的
# 是同一条机制），并打印每步的焦点句柄。
#
# # 判据
#
# **落点互不相同**（且最好回到起点，说明会循环）。若全部相同 —— 那正是
# 本轮抓到的真缺陷形态（`tab_index` 齐了但没配 `track_focus`，
# 登记不进顺序表，`focus_next` 原地不动）。
#
# # 不抢焦点
#
# 探针直接调 API，不需要窗口在最前台，符合项目约定（效率规范第零条）。
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
STEPS=6
[ "${1:-}" = "--steps" ] && STEPS="${2:?--steps 需要一个数}"

APP="dist/NEO.app/Contents/MacOS/neo"
if [ ! -x "$APP" ]; then
  echo "❌ 找不到 $APP —— 先跑：bash scripts/make-app.sh"
  exit 1
fi

log=$(mktemp -t neo-kbd-XXXXXX.log)
# 带一个面板打开：这样顺序表里除了状态栏还有面板里的元素，覆盖面更真实
NEO_GUI_TAB="$STEPS" NEO_GUI_PANEL=files \
  "$APP" desktop --gpui --provider mock --workspace "$ROOT" >"$log" 2>&1 &
pid=$!
# 条件等待：出现探针那行即就绪（有总超时上限）
deadline=$((SECONDS + 30))
while [ "$SECONDS" -lt "$deadline" ]; do
  grep -q 'tab-probe' "$log" 2>/dev/null && break
  sleep 1
done
kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null

line=$(grep -m1 'tab-probe' "$log" 2>/dev/null)
rm -f "$log"
if [ -z "$line" ]; then
  echo "❌ 30 秒内没拿到探针输出"
  exit 1
fi
echo "$line" | sed 's/^/  /'

# 抽出落点列表（`→ ["a", "b", ...]`）
landings=$(printf '%s' "$line" | sed -n 's/.*→ \[\(.*\)\].*/\1/p' | tr ',' '\n' | sed 's/[ "]//g' | grep -v '^$')
total=$(printf '%s\n' "$landings" | grep -c . || true)
distinct=$(printf '%s\n' "$landings" | sort -u | grep -c . || true)

echo
echo "  走 $total 步，落点互异数：$distinct"
if [ "$distinct" -le 1 ]; then
  echo "  ❌ 所有落点相同 —— Tab 顺序没生效。"
  echo "     最常见原因：元素有 tab_index 但**没有 track_focus**（登记不进顺序表）。"
  echo "     跑 docs/neo-plan/05-验证/checks/check_a11y.py 看是不是这条。"
  exit 1
fi
echo "  ✅ 键盘可达：Tab 顺序生效（落点互异）"
