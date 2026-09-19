#!/usr/bin/env bash
# NEO 全套门禁入口。
#
# 七部分：
#   0. 预检：cargo 可执行 **且是钉版的那把**（解析失败 / 版本选错都会让后续门禁失去意义）
#   1. Rust 工程门禁：cargo test（含内核 conformance、内存有界性、SPI conformance）
#   2. 架构与协议守卫：docs/neo-plan/05-验证/ 的 Python 检查（依赖方向、协议确定性、
#      会话格式、配置层叠、模式矩阵、SPI 合规、UI 分层、可提取性、许可证）
#   3. 工具链卫生：零 warning（warning 是未来错误的温床）
#   4. 执行效率规范：固定盲等 / 重复拉取 / 无退出轮询（ai-efficiency-rules）
#   5. shell 卫生：变量后紧跟非 ASCII（bash 3.2 会把中文标点吃进变量名）
#   6. CI 失败摘要：注解通道是否仍能读到真因
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

# ── 0. 预检：cargo 能不能跑 + 是不是钉版的那把 ──────────────────────────
#
# 区分「环境坏了」与「代码没过」。两类环境故障都实际发生过：
#   ① rust-toolchain.toml 不是合法 TOML —— rustup 会让本仓库内每一条 cargo
#      命令直接失败；
#   ② PATH 上先命中的是**另一把 cargo**（本机实测：Homebrew 的
#      /opt/homebrew/bin 排在 ~/.cargo/bin 前面）—— 此时 `cargo --version`
#      正常退出，旧 cargo 却解析不了依赖树（树里有包要求 edition2024，即
#      cargo ≥ 1.85），报出来的是 "failed to parse manifest ... feature
#      edition2024 is required"。它**看起来像编译错误，实为工具链选错**，
#      且会同时污染 4 个门禁（测试 / warning / 可提取性 / 第三方清单）——
#      上游日志里出现的正是这一串连带失败。
# 两种都不预检的话，第 2 段「零 warning」会因为输出里没有 warning 而报
# 「✅ 无 warning」—— 那是假通过（只有编译真的跑起来，warning 计数才有意义）。
#
# 判定逻辑**不在本文件**：它被别的脚本共用（make-app.sh / 将来的打包脚本），
# 抄一份必然漂移，故抽到 lib 里。库文件用的是"失败即返回非零 + stderr 说明"，
# 门禁这边需要一个布尔量，故包一层。
. "$ROOT/scripts/lib-rust-toolchain.sh"
CARGO_OK=0
if command -v cargo >/dev/null 2>&1 && [ "$(command -v cargo)" != "$HOME/.cargo/bin/cargo" ]; then
  # 只在"确实换过"时提示（库函数内部已把 PATH 优先级调整好）
  echo "  ℹ️  当前 PATH 先命中的是 $(command -v cargo)，已优先改用 rustup shim：\$HOME/.cargo/bin/cargo"
fi
if ensure_pinned_cargo; then
  CARGO_OK=1
  # 库函数只报错不报成功（非门禁场景不需要那行输出），这里补一句让人安心。
  VER="$(cargo --version 2>/dev/null | grep -m1 '^cargo ' | awk '{print $2}')"
  echo "  ✅ 工具链：cargo ${VER:-未知}（与 rust-toolchain.toml 一致）"
else
  # 库函数已把原因与修法打到 stderr，这里补一句门禁口径。
  echo "  ⚠️  cargo 未通过预检 —— Rust 门禁整体判失败（是工具链/环境问题，非代码问题）"
  CARGO_OK=0
fi

# ── 1. Rust 测试（内核 conformance 是主门禁）────────────────────────────
hr; echo "#  Rust: cargo test --workspace（内核 / 内存 / SPI conformance）"; hr
if [ "$CARGO_OK" -eq 1 ]; then
  ( cd "$ROOT" && cargo test --workspace ) || fail=$((fail+1))
else
  echo "  [SKIP] cargo 未通过预检 —— Rust 门禁未执行（见上方预检信息）。"
  fail=$((fail+1))
fi

# ── 2. 警告即错误（工具链卫生）──────────────────────────────────────────
#
# 关键：cargo 必须**成功退出**才有资格谈 warning 计数。编译都没跑起来时
# 输出里自然没有 warning，直接数 0 会把「没编译」误判成「零 warning」。
hr; echo "#  Rust: 零 warning 检查"; hr
if [ "$CARGO_OK" -eq 1 ]; then
  ( cd "$ROOT" && cargo check --workspace --all-targets ) >/tmp/neo-check.log 2>&1
  check_rc=$?
  if [ "$check_rc" -ne 0 ]; then
    echo "  ❌ cargo check 未成功（exit ${check_rc}）—— 无法判定 warning，按失败处理"
    head -20 /tmp/neo-check.log | sed 's/^/     /'
    fail=$((fail+1))
  else
    warn_out="$(grep -c '^warning' /tmp/neo-check.log || true)"
    if [ "${warn_out:-0}" -eq 0 ]; then
      echo "  ✅ 无 warning"
    else
      echo "  ❌ 有 $warn_out 条 warning —— warning 是未来错误的温床，请清零"
      grep -A4 '^warning' /tmp/neo-check.log | head -40 | sed 's/^/     /'
      fail=$((fail+1))
    fi
  fi
else
  echo "  [SKIP] cargo 未通过预检 —— 零 warning 检查未执行（见上方预检信息）。"
  fail=$((fail+1))
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
    "check_spi_conformance.py|SPI 合规：契约 / >=2 后端 / conformance" \
    "check_ui_layering.py|UI 栈守卫：开源边界 / 单一 GPUI pin / 行为层解耦" \
    "check_extractable.py|可提取性：可开源集复制到空仓能否编译（结构检查）" \
    "check_licenses.py|许可证合规：依赖声明的许可是否都在允许列表内" ; do
    script="${entry%%|*}"; desc="${entry##*|}"
    hr; echo "#  $desc"; hr
    py=python3; command -v "$py" >/dev/null 2>&1 || py=python
    ( cd "$PLAN_CHECKS" && "$py" "checks/$script" ) || fail=$((fail+1))
  done

  # 第三方许可证清单是否与当前依赖一致（防止清单腐烂）。
  # 它不属于 `checks/` 那一组（那组是"守卫"，这个是"交付物是否最新"），
  # 但对开源产物同样重要：清单过期等于给使用者一份错的信息。
  hr; echo "#  第三方许可证清单是否最新"; hr
  py=python3; command -v "$py" >/dev/null 2>&1 || py=python
  ( cd "$ROOT" && "$py" scripts/gen_third_party_licenses.py --check ) || fail=$((fail+1))
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
  echo "  [SKIP] 未找到 ${AUDIT}（skill 未安装？见 docs/ai-efficiency-rules/）"
fi

# ── 5. shell 多字节变量名（bash 3.2 会把中文标点吃进变量名）─────────────
#
# 这类写法语法合法（`bash -n` 查不出），只在真跑时炸成
# `V）: unbound variable`，且本项目已复发 4 次（publish-npm / install /
# verify / release.yml 各一次），故固化成门禁。
hr; echo "#  shell 卫生：变量后紧跟非 ASCII（bash 3.2 坑）"; hr
MB="$ROOT/scripts/check_shell_multibyte.sh"
if [ -f "$MB" ]; then
  if bash "$MB"; then :; else fail=$((fail+1)); fi
else
  echo "  [SKIP] 未找到 ${MB}"
fi

# ── 6. CI 失败摘要（可读性守卫）─────────────────────────────────────────
#
# 为什么它也进门禁：**读不到原因的 CI = 白跑一轮**。失败日志的注解通道
# 有两个硬约束（每个 step 只保留前 10 条 error 注解；日志里满是
# `Compiling thiserror` / `test result: ok. 0 failed` 这类"像错误的正常行"），
# 一旦有人把模式改回 `grep -i 'error|failed'`，配额会被噪声吃光，
# 真因又被静默丢弃。故用自检装置把它钉住（含链接期失败这类不带 `^error`
# 前缀的行 —— 曾整条 Linux 依赖树缺库因此漏判）。
hr; echo "#  CI 失败摘要：注解通道是否仍能读到真因"; hr
DIGEST="$ROOT/scripts/ci-failure-digest.sh"
if [ -f "$DIGEST" ]; then
  if bash "$DIGEST" --selftest; then :; else fail=$((fail+1)); fi
else
  echo "  [SKIP] 未找到 ${DIGEST}"
fi

echo
echo "============================================================"
if [ "$fail" -eq 0 ]; then echo "✅ 全部门禁通过"; exit 0
else echo "❌ 有 $fail 组未通过"; exit 1; fi