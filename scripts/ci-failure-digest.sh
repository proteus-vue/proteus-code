#!/usr/bin/env bash
# 把失败的测试日志压缩成**可读的** GitHub 注解（`::error::` / `::warning::`）。
#
# ── 为什么需要这个脚本（每条都是踩过的坑）─────────────────────────────
#
# 1. **注解通道有硬上限：每个 step 只保留「前 10 条」error 注解**，其余静默丢弃。
#    所以必须**按价值排序**、总量受控。"无条件逐行发"等于把预算烧在最先出现
#    的行上 —— 实测发 45 条，API 只还回 10 条，且全是噪声（见下）。为把预算
#    翻倍，低优先级的兜底信息改走 `::warning::`（另一条独立的 10 条配额）。
#
# 2. **日志里大量"长得像错误的正常行"**：
#      `   Compiling thiserror v2.0.20`      ← 命中 `-i` 的 error
#      `test result: ok. 0 passed; 0 failed` ← 命中 `-i` 的 failed
#    用 `grep -i 'error|failed'` 时，前 10 条预算会被这类噪声**全部吃光**，
#    真正的失败行一条都发不出去 —— 表现就是"CI 红了，但我读不到原因"。
#    本脚本的模式**一律行首锚定、大小写敏感**，从根上排除这两类噪声。
#
# 3. **注解消息里的换行会被 API 截断**（多行注解只留第一行），故每条消息必须
#    压成**单行**（内部换行用 `⏎` 表示）。
#
# 4. **注解是唯一免认证可读的通道**：job 日志
#    （`/actions/jobs/<id>/logs`）与 STEP SUMMARY 都要 admin 权限（实测 403
#    "Must have admin rights to Repository."）。读不到 = 白跑一轮 CI。
#
# ── 用法 ───────────────────────────────────────────────────────────────
#   bash scripts/ci-failure-digest.sh <日志文件> [每通道上限=10]
#
# 输出即 GitHub 工作流命令，可直接在 workflow 里 `bash scripts/ci-failure-digest.sh log`。
set -uo pipefail

# ── 自检（`--selftest`）：守卫要有牙齿 ─────────────────────────────────
#
# 本脚本要防的退化很具体：有人图方便把模式改回 `grep -i 'error|failed'`，
# 于是噪声（`Compiling thiserror` / `test result: ok. 0 failed`）再次吃光
# 注解配额，真因又读不到。故用固定装置钉住三件事：
#   A. 有真失败时，**失败用例名**必须出现在输出里；
#   B. 噪声行（Compiling thiserror / test result: ok）**不得**占 error 通道；
#   C. 链接期失败（不带 `^error` 前缀）也必须被发出来。
# 由 `scripts/verify.sh` 调用。
if [ "${1:-}" = "--selftest" ]; then
  tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
  fails=0

  cat > "$tmp/noise_plus_failure.log" <<'FIX'
   Compiling thiserror v2.0.20
   Compiling quick-error v1.2.3
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
running 1 test
test ime_preedit_input ... FAILED
thread 'ime_preedit_input' panicked at crates/neo-ui/tests/ime_regression.rs:70:5:
assertion failed
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
FIX
  out="$(bash "$0" "$tmp/noise_plus_failure.log" 10)"
  errs="$(printf '%s\n' "$out" | grep -c '^::error' || true)"

  # A. 失败用例名必须出现
  if printf '%s' "$out" | grep -q 'ime_preedit_input'; then
    echo "  ✅ A 真失败被发出（含用例名）"
  else
    echo "  ❌ A 真失败**没有**被发出 —— 真因又读不到了"; fails=$((fails+1))
  fi
  # B. 噪声不得进 error 通道
  if printf '%s\n' "$out" | grep '^::error' | grep -qE 'Compiling thiserror|test result: ok'; then
    echo "  ❌ B 噪声（Compiling thiserror / test result: ok）占了 error 通道"; fails=$((fails+1))
  else
    echo "  ✅ B 噪声未占用 error 通道"
  fi
  # C. error 通道总量不超上限（GitHub 只保留前 10 条）
  if [ "${errs:-0}" -le 10 ]; then
    echo "  ✅ C error 注解数 ${errs} ≤ 10"
  else
    echo "  ❌ C error 注解数 ${errs} 超过 10 —— 超出的会被 GitHub 静默丢弃"; fails=$((fails+1))
  fi

  cat > "$tmp/link_failure.log" <<'FIX'
   Compiling neo-host-gpui v0.1.0
error: linking with `cc` failed: exit status: 1
  = note: rust-lld: error: unable to find library -lxkbcommon-x11
error: could not compile `neo-host-gpui` (bin "neo") due to 1 previous error
FIX
  out2="$(bash "$0" "$tmp/link_failure.log" 10)"
  if printf '%s' "$out2" | grep -q 'unable to find library'; then
    echo "  ✅ C 链接期失败被发出（该类行不带 ^error 前缀）"
  else
    echo "  ❌ C 链接期失败被漏掉 —— 依赖树缺库会又一次读不到"; fails=$((fails+1))
  fi

  # 缺日志本身也要成为一条可读诊断（否则失败会表现为"零注解"）
  out3="$(bash "$0" "$tmp/does-not-exist.log" 10)"
  if printf '%s' "$out3" | grep -q '^::error'; then
    echo "  ✅ D 缺日志时会发出诊断注解"
  else
    echo "  ❌ D 缺日志时静默无输出"; fails=$((fails+1))
  fi

  if [ "$fails" -eq 0 ]; then echo "✅ ci-failure-digest 自检通过"; exit 0; fi
  echo "❌ ci-failure-digest 自检未通过（$fails 项）"; exit 1
fi

LOG="${1:-}"
LIMIT="${2:-10}"

if [ -z "$LOG" ] || [ ! -f "$LOG" ]; then
  # 没有日志本身就是一条必须被读到的诊断
  echo "::error title=诊断::日志不存在或不可读: ${LOG:-<未传参>}"
  exit 0
fi

PLAIN="$(mktemp)"
trap 'rm -f "$PLAIN"' EXIT
# ⚠️ 先去 ANSI 再去 grep：带色日志的行其实以 \x1b[1m 开头，
# 行首锚点 `^` 会因此永远匹配不上（为此白跑过一轮）。
sed -E 's/\x1b\[[0-9;]*m//g' "$LOG" > "$PLAIN"

_n_err=0
_n_warn=0
_seen=""  # 已发消息指纹：避免"尾部"兜底把前面的行再发一遍
_push() { # $1=级别  $2=标题  $3=消息
  local lvl="$1" title="$2" msg fp
  msg="$(printf '%s' "$3" | tr '\n\r\t' ' ' | sed -E 's/  +/ /g; s/^ //; s/ $//' | cut -c1-800)"
  [ -z "$msg" ] && return 0
  fp="$(printf '%s' "$msg" | cksum)"
  case "$_seen" in *"|$fp|"*) return 0 ;; esac
  if [ "$lvl" = error ]; then
    [ "$_n_err" -ge "$LIMIT" ] && return 0
    _n_err=$((_n_err + 1))
  else
    [ "$_n_warn" -ge "$LIMIT" ] && return 0
    _n_warn=$((_n_warn + 1))
  fi
  _seen="${_seen}|${fp}|"
  echo "::${lvl} title=${title}::${msg}"
}

# ── error 通道：真正的失败信号，按价值从高到低（上限内先到先得）──────

# 1) 失败用例名 —— 最直接的信息，一条注解装下全部（空格分隔）
names="$(grep -aE '^test .* \.\.\. FAILED' "$PLAIN" \
  | sed -E 's/^test ([^ ]+) .*/\1/' | sort -u | head -25 | paste -sd' ' -)"
[ -n "$names" ] && _push error "失败用例" "$names"

# 2) panic 区块 —— "为什么红"在 panic 行**之后**几行（断言消息、left/right）。
#    只发 panic 行会看得到崩溃点却看不到差异，故带上后续 3 行。
while IFS= read -r line; do
  _push error "panic" "$line"
done < <(awk '/panicked at/ { l=$0; for (i=0; i<3 && (getline nxt)>0; i++) l = l " ⏎ " nxt; print l }' "$PLAIN" | head -3)

# 3) 编译错误（`error[E0xxx]` / `error:`）—— 带上紧随的 `-->` 定位行
while IFS= read -r line; do
  _push error "编译错误" "$line"
done < <(awk '/^error(\[|:)/ { l=$0; if ((getline nxt)>0 && nxt ~ /^ *-->/) l = l " ⏎ " nxt; print l }' "$PLAIN" | head -5)

# 4) 链接期失败 —— 与编译错误**不同源**：`cargo check` 不链接，故只在 test
#    阶段暴露（曾因只看编译错误而漏判整条 Linux 依赖树缺库）。这类行的
#    关键信息常在 `= note:` 里，而 note 行不带 `error` 前缀。
while IFS= read -r line; do
  _push error "链接失败" "$line"
done < <(grep -aE 'could not compile|error: linking with|unable to find library|undefined reference to' "$PLAIN" | head -3)

# ── warning 通道：兜底与元信息（不与真正失败抢 error 配额）────────────

# 元信息：退出码/日志规模/磁盘。用来区分"没进诊断分支"与"进了但输出被吞"。
# 走 warning，因为它是**背景**而非失败本身。规模优先用调用方传入的值
# （workflow 里已算过），缺省再自行统计。
if [ -n "${NEO_CI_EXIT:-}" ]; then
  _l="${NEO_CI_LINES:-$(wc -l < "$PLAIN" | tr -d ' ')}"
  _b="${NEO_CI_BYTES:-$(wc -c < "$PLAIN" | tr -d ' ')}"
  _push warning "诊断" "exit=${NEO_CI_EXIT} lines=${_l} bytes=${_b} 磁盘剩余=${NEO_CI_DISK:-?}"
fi

# 兜底：尾部非空行。既非用例失败也非编译错误时（例如被信号杀死/OOM），
# 尾部是唯一线索。
while IFS= read -r line; do
  _push warning "尾部" "$line"
done < <(grep -av '^[[:space:]]*$' "$PLAIN" | tail -6)
