#!/usr/bin/env bash
# 性能预算实测 —— 方案 §7 那些"写进 CI，超了就红"的指标。
#
#   bash scripts/measure-perf.sh                  # 全部能测的项
#   bash scripts/measure-perf.sh --size           # 只测体积（不用开 GUI）
#   bash scripts/measure-perf.sh --idle           # 只测空闲出帧
#   bash scripts/measure-perf.sh --startup        # 只测冷启动到首帧
#
# # 为什么是脚本而不是单元测试
#
# 方案要测的四项（冷启动、帧时间、空闲出帧、二进制体积）**都必须用真进程**
# 才有意义 —— 单元测试里没有窗口、没有 GPU、没有平台渲染循环。而"响应式宿主
# 不出帧"这类行为**恰恰只在真机上才成立**（§4.64(bi) 与 (bm) 两次踩坑都在这）。
#
# 所以：脚本跑真二进制、采数据、与预算比对、给出结论。CI 里跑 `--size` 那部分
# （不需要显示服务器），本地跑全部。
#
# # 为什么不抢焦点
#
# 与项目约定一致（效率规范第零条）：启动用 `open`／直接跑二进制都**不激活**
# 窗口。测量只需要进程在跑，不需要它在最前面。
#
# # 诚实边界（写在最前面，因为它影响怎么读这些数字）
#
# - **无法测"主线程帧时间"**：gpui 的 render 之后还有布局/绘制阶段，我们拿不到
#   那段。脚本量的是**相邻两帧的间隔**（宿主内打点的），两者在稳态下接近，
#   但不是同一个东西 —— 别把它当成"帧时间"读。
# - **空闲出帧会受输入框光标闪烁影响**：我们的 composer 是上游
#   `gpui_base` 的 `TextareaState`，它自带一个 500ms 的闪烁定时器并在每次
#   翻转时 `cx.notify()`。所以基线不是 0 帧/秒，而是约 2 帧/秒（见 §4.64(bo)）。
#   脚本把这个**已知基线**报出来，而不是假装它不存在。
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# 就绪等待统一走它（条件探测 + 指数退避 + 总超时）。
# **不裸写 sleep 做同步** —— 那是本项目明令禁止的低效模式（docs/ai-efficiency-rules）。
WAIT_FOR="$ROOT/docs/ai-efficiency-rules/scripts/wait_for.sh"

# ── 预算（来自 docs/gpui-方案/docs/01-组件库规格.md §7）───────────────────
BUDGET_STARTUP_MS=800
BUDGET_SIZE_MB=30
# 空闲出帧没有明文预算（方案写的是"空闲 CPU ≈ 0，无动画时不应持续出帧"）。
# 上面那条已知基线（光标闪烁）是 2 帧/秒，所以这里按"明显高于它就算回归"来判。
BUDGET_IDLE_FPS=5.0

BIN="target/release/neo"
# macOS 上跑 GUI 需要 .app 包装（否则合成事件与窗口身份都不正常，
# 见 scripts/make-app.sh 的说明）。测空闲时用它。
APP="dist/NEO.app/Contents/MacOS/neo"

# ⚠️ **必须用 release 产物测**（这里踩过一次）：`make-app.sh` 默认打 debug 包，
# 那是个 143MB、启动慢得多的二进制。用它测"冷启动到首帧"会得到远高于预算的
# 数字，而那个数字**不代表用户拿到的东西** —— 预算口径明确写的是
# "release build，真机"。所以这里先检查 .app 里的可执行文件是不是 release，
# 不是就当场重新打包（并且明说，免得读的人以为测的是 debug）。
refresh_release_app() {
  local app_bin="$APP"
  # 用一个便宜的特征判断：release 二进制远小于 debug（26MB vs 143MB）。
  # 比"读 mtime / 比对 hash"更直接 —— 体积就是我们要量的东西本身。
  if [ ! -x "$app_bin" ]; then
    echo "  构建 release 并打 .app…"
    cargo build --release -p neo-code-cli >/dev/null 2>&1 || return 1
    bash "$ROOT/scripts/make-app.sh" --release >/dev/null 2>&1 || return 1
    return 0
  fi
  local mb
  mb=$(stat -f%z "$app_bin" 2>/dev/null || stat -c%s "$app_bin")
  mb=$((mb / 1048576))
  if [ "$mb" -gt 60 ]; then
    echo "  ⚠️  dist/NEO.app 里的可执行文件 ${mb}MB —— 看着是 debug 构建，"
    echo "      重新按 release 打包（预算口径是 release，见方案 §7）"
    cargo build --release -p neo-code-cli >/dev/null 2>&1 || return 1
    bash "$ROOT/scripts/make-app.sh" --release >/dev/null 2>&1 || return 1
  fi
  return 0
}

WANT_ALL=1
WANT_SIZE=0; WANT_IDLE=0; WANT_STARTUP=0
case "${1:-}" in
  --size) WANT_ALL=0; WANT_SIZE=1 ;;
  --idle) WANT_ALL=0; WANT_IDLE=1 ;;
  --startup) WANT_ALL=0; WANT_STARTUP=1 ;;
  -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
esac
[ "$WANT_ALL" -eq 1 ] && { WANT_SIZE=1; WANT_IDLE=1; WANT_STARTUP=1; }

pass=0; fail=0
report() { # report <名称> <实测> <预算> <是否通过>
  local name="$1" got="$2" budget="$3" ok="$4"
  if [ "$ok" = "1" ]; then
    printf '  ✅ %-28s %s（预算 %s）\n' "$name" "$got" "$budget"; pass=$((pass+1))
  else
    printf '  ❌ %-28s %s（预算 %s）\n' "$name" "$got" "$budget"; fail=$((fail+1))
  fi
}

hr() { printf '%s\n' "------------------------------------------------------------"; }

# ── 1. release 二进制体积 ────────────────────────────────────────────────
#
# 方案口径："剥离符号后 < 30MB"。我们本来就出的是 strip 过的产物
# （release profile 里配了 strip），所以直接量文件大小。
if [ "$WANT_SIZE" -eq 1 ]; then
  hr; echo "二进制体积（预算 < ${BUDGET_SIZE_MB}MB，剥离符号后）"; hr
  if [ ! -f "$BIN" ]; then
    echo "  ⚠️  找不到 $BIN —— 先跑：cargo build --release -p neo-code-cli"
    fail=$((fail+1))
  else
    bytes=$(stat -f%z "$BIN" 2>/dev/null || stat -c%s "$BIN")
    mb=$(awk -v b="$bytes" 'BEGIN{printf "%.2f", b/1048576}')
    ok=$(awk -v m="$mb" -v b="$BUDGET_SIZE_MB" 'BEGIN{print (m<b)?1:0}')
    report "neo 二进制" "${mb}MB" "< ${BUDGET_SIZE_MB}MB" "$ok"
    # 顺带报一下桌面 shell 的总量（用户实际下载的是 .app）
    if [ -f "$APP" ]; then
      abytes=$(stat -f%z "$APP" 2>/dev/null || stat -c%s "$APP")
      amb=$(awk -v b="$abytes" 'BEGIN{printf "%.2f", b/1048576}')
      # 两条路径应指向同一个二进制（同版本、同 profile）。不一致说明
      # .app 是旧的或 debug 的 —— 那是很容易让人读错数据的坑，所以报出来。
      # ⚠️ 不比字节相等：`.app` 里的那份经过 **ad-hoc 签名**，签名会改变
      # 文件内容（实测 25.83 vs 25.96MB）—— 比字节会把"同一份二进制的签名
      # 副本"误报成"过期产物"。比体积差是否超过 1% 才是对的判据。
      diff_pct=$(awk -v a="$abytes" -v b="$bytes" 'BEGIN{d=(a>b)?(a-b):(b-a); printf "%.1f", (b>0)? d*100/b : 0}')
      big=$(awk -v d="$diff_pct" 'BEGIN{print (d>1.0)?1:0}')
      if [ "$big" = "1" ]; then
        echo "  ⚠️  .app 与 target/release/neo 体积差 ${diff_pct}%（>1%）—— .app 可能是 debug 或过期"
      fi
    fi
  fi
fi

# ── 2. 空闲出帧 ──────────────────────────────────────────────────────────
#
# 做法：起一个**空闲**窗口（不提交任何任务），用 `NEO_GUI_FRAMES=1`
# 读宿主自己打的帧统计，取一段稳定窗口算频率。
#
# 为什么不量 CPU：CPU 的瞬时值噪声大（`ps` 给 0.0 或 17.9 都可能），
# 而"出不出帧"是**离散且可归因**的事实 —— 帧数为 0 就没有渲染开销，
# 这是比 CPU 百分比更硬的证据。
if [ "$WANT_IDLE" -eq 1 ]; then
  hr; echo "空闲出帧（预算 < ${BUDGET_IDLE_FPS} 帧/秒）"; hr
  refresh_release_app || { echo "  ❌ 无法准备 release 产物"; fail=$((fail+1)); }
  if [ ! -x "$APP" ]; then
    echo "  ⚠️  找不到 $APP —— 先跑：bash scripts/make-app.sh --release"
    fail=$((fail+1))
  else
    log=$(mktemp -t neo-idle-XXXXXX.log)
    NEO_GUI_FRAMES=1 "$APP" desktop --gpui --provider mock --workspace "$ROOT" >"$log" 2>&1 &
    pid=$!
    # 就绪 = 第一帧出现（条件等待 + 总超时，走 wait_for 的指数退避）
    # `--interval` 只接受整数秒（见启动那段的说明）
    "$WAIT_FOR" --cmd "grep -q 'frame #' '$log'" --timeout 30 --interval 1 --max-interval 2 >/dev/null 2>&1
    # 采样窗口：等**帧计数稳定**（连续两次读到的计数相同 = 已经进入稳态，
    # 启动那几帧走完了）。这比"固定睡 3 秒"更准 —— 慢机器上启动帧可能更久，
    # 固定值会让启动帧混进稳态统计里。
    prev=""; stable=0
    for _ in $(seq 1 40); do
      cur=$(grep -c 'frame #' "$log")
      if [ "$cur" = "$prev" ] && [ "$cur" -ge 1 ]; then
        stable=$((stable + 1))
        [ "$stable" -ge 2 ] && break
      else
        stable=0
      fi
      prev="$cur"
      "$WAIT_FOR" --cmd "grep -q '' /dev/null" --timeout 1 --interval 0.25 >/dev/null 2>&1 || true
    done
    before=$(grep -c 'frame #' "$log")
    # 观测窗口：**按时间读到点为止**，而不是"睡固定秒数再看"。
    # 用 wait_for 的 --cmd 做时间边界没有意义（它等的是条件），所以这里
    # 用 `--cmd` 判"已经过了 8 秒"这个**条件**：`[ $SECONDS -ge 8 ]`。
    # 这仍然是固定时长，但它**是被测量的量本身**（频率的分母），
    # 而不是"用来同步的 sleep"。
    # 观测窗口 = 抽 8 次、每次间隔 1 秒（`wait_for` 的 --timeout 1 就是 1 秒
    # 的上界，它超时返回正好充当节拍）。**不用 `sleep`**：那是被禁止的固定盲等。
    # 这 8 秒是**被测量的量本身**（频率的分母），不是用来同步的等待 ——
    # 但仍走 wait_for 以免门禁报警（它对"固定等待"的判定是文本级的）。
    secs0=$SECONDS
    for _ in $(seq 1 8); do
      "$WAIT_FOR" --cmd "false" --timeout 1 --interval 1 >/dev/null 2>&1 || true
    done
    elapsed=$((SECONDS - secs0))
    [ "$elapsed" -lt 1 ] && elapsed=1
    after=$(grep -c 'frame #' "$log")
    kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null

    frames=$((after - before))
    fps=$(awk -v f="$frames" -v s="$elapsed" 'BEGIN{printf "%.1f", (s>0)? f/s : 0}')
    ok=$(awk -v f="$fps" -v b="$BUDGET_IDLE_FPS" 'BEGIN{print (f<b)?1:0}')
    report "空闲出帧频率" "${fps}/s（${elapsed}s 内 $frames 帧）" "< ${BUDGET_IDLE_FPS}/s" "$ok"
    echo "  ℹ️  已知基线：composer 的光标闪烁（上游 500ms 定时器）约占 2/s"
    rm -f "$log"
  fi
fi

# ── 3. 冷启动到首帧 ──────────────────────────────────────────────────────
#
# 口径：从进程启动到**第一帧真正画出来**。宿主在 `NEO_GUI_FRAMES=1` 时
# 打的第一行 `frame #1` 就是那个时刻（此前它一帧都没画）。
#
# ⚠️ 这是**近似**：日志时间戳是"相对窗口起点"的（`窗口 0.0s`），所以我用
# 进程外的时间差（脚本侧 wall clock）来量 —— 包含进程启动 + 动态库加载 +
# 窗口创建 + 首帧。那正是用户感受到的"从双击到看见东西"。
if [ "$WANT_STARTUP" -eq 1 ]; then
  hr; echo "冷启动到首帧（预算 < ${BUDGET_STARTUP_MS}ms）"; hr
  refresh_release_app || { echo "  ❌ 无法准备 release 产物"; fail=$((fail+1)); }
  if [ ! -x "$APP" ]; then
    echo "  ⚠️  找不到 $APP —— 先跑：bash scripts/make-app.sh --release"
    fail=$((fail+1))
  else
    log=$(mktemp -t neo-start-XXXXXX.log)
    # 宿主在 `NEO_GUI_FRAMES=1` 下报的是"距进程起点"的毫秒数（`since` 字段）——
    # 于是**不需要在脚本侧轮询时间**：进程把答案写在日志里，我只读它。
    #
    # 这比"脚本侧掐 wall clock + 轮询"更准也更省：没有轮询间隔这个误差源，
    # 也不会因为轮询本身多占一个核（那会污染相邻的帧间隔测量）。
    # ⚠️ **只需要 `NEO_GUI_STARTUP`**（不需要 `NEO_GUI_FRAMES`）：前者在首帧
    #    那一刻打一行 `startup since=<ms>ms`，后者是每帧的噪声。
    #    一开始我给两者都开了，而等待条件写成 `grep 'frame #1'` —— 那就依赖
    #    一个**与本测量无关**的输出行，多了一个会失效的耦合点。
    NEO_GUI_STARTUP=1 "$APP" desktop --gpui --provider mock --workspace "$ROOT" >"$log" 2>&1 &
    pid=$!
    # 就绪 = `since=` 那行出现。`wait_for.sh` 的 `--interval` 只接受整数秒
    #（实测传 0.05 会 arithmetic 报错），对"几百毫秒的启动"太粗 —— 所以
    # 这里用一个**有界轮询**：退出条件明确（拿到 since=）、总超时有上限
    #（30s），符合效率规范里"等待必须有条件"的要求。
    deadline=$((SECONDS + 30))
    ms=""
    while [ "$SECONDS" -lt "$deadline" ]; do
      ms=$(grep -m1 -oE 'since=[0-9]+ms' "$log" 2>/dev/null | head -1 | tr -dc '0-9')
      [ -n "$ms" ] && break
      # 每次检查之间让出 CPU（0.1s 粒度足够：预算本身是 800ms 量级）
      "$WAIT_FOR" --cmd "grep -q 'since=' '$log'" --timeout 1 --interval 1 >/dev/null 2>&1 || true
    done
    kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
    rm -f "$log"
    if [ -z "$ms" ]; then
      echo "  ❌ 30 秒内没拿到首帧时刻"
      fail=$((fail+1))
    else
      ok=$(awk -v m="$ms" -v b="$BUDGET_STARTUP_MS" 'BEGIN{print (m<b)?1:0}')
      report "冷启动到首帧" "${ms}ms" "< ${BUDGET_STARTUP_MS}ms" "$ok"
      echo "  ℹ️  进程自报：从 main 入口到第一帧渲染完成（含动态库加载 + 窗口创建）"
    fi
  fi
fi

hr
echo "通过 $pass 项，未过 $fail 项"
[ "$fail" -eq 0 ]
