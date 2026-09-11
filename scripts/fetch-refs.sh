#!/usr/bin/env bash
# fetch-refs.sh —— 拉取**参考项目源码**（opencode / mimo code 等）
#
# 为什么需要这个脚本：`docs/opencode-parity.md`（对齐规格）是从 opencode 源码
# 提取的，而源码缓存落在 `.cache/` 里、**被 .gitignore 忽略** ——
# 所以**换一台电脑就没有这些源码**，规格文档里的文件路径全都指不到东西。
# 这个脚本把"重新拉取"从口头知识变成一条命令。
#
# 用法:
#   bash scripts/fetch-refs.sh            # 拉全部参考项目
#   bash scripts/fetch-refs.sh --list     # 只看当前缓存状态
#   PROXY=http://127.0.0.1:7897 bash scripts/fetch-refs.sh   # 显式指定代理
#
# 新机器上要做的三件事（脚本会自动尝试前两件）:
#   1. 能连上 github（可能需要代理 —— 见 detect_proxy）
#   2. 跑本脚本
#   3. 若 MiMo 侧要对照，另需装 `mimo`（它是编译产物，无源码；见文件末尾）

set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CACHE_FETCH="$ROOT/docs/ai-efficiency-rules/scripts/cache_fetch.sh"

if [[ ! -f "$CACHE_FETCH" ]]; then
  echo "找不到 cache_fetch.sh：$CACHE_FETCH" >&2
  exit 1
fi

# 依赖的外部参考项目清单。
# 格式: <名字>|<repo>|<ref>|<subdir>
REFS=(
  "opencode|https://github.com/sst/opencode.git|dev|packages/tui"
)

cache_dir() { echo "$ROOT/.cache/ai-external/github/$1-${2}"; }

# ── 代理探测 ────────────────────────────────────────────────────────────
# 为什么必须自动探：项目机器在国内，github 直连会以
# "Error in the HTTP2 framing layer / Couldn't connect" 失败，
# 而 git 不会读系统代理设置。之前是靠人工试出 7897 端口，这里固化成逻辑。
detect_proxy() {
  if [[ -n "${PROXY:-}" ]]; then echo "$PROXY"; return; fi
  # 环境里已有的代理优先
  for v in HTTPS_PROXY https_proxy ALL_PROXY all_proxy; do
    if [[ -n "${!v:-}" ]]; then echo "${!v}"; return; fi
  done
  # 常见本地代理端口（Clash/Surge/V2Ray 等）
  for p in 7897 7890 7891 1080 1087 8888 8118 10809; do
    if curl -sS -x "http://127.0.0.1:$p" --max-time 6 -o /dev/null https://github.com 2>/dev/null; then
      echo "http://127.0.0.1:$p"; return
    fi
  done
  echo ""   # 探测不到：让 git 直连试试
}

PROXY_URL="$(detect_proxy)"
if [[ -n "$PROXY_URL" ]]; then
  echo "[refs] 使用代理：$PROXY_URL"
  export HTTP_PROXY="$PROXY_URL" HTTPS_PROXY="$PROXY_URL"
  export http_proxy="$PROXY_URL" https_proxy="$PROXY_URL"
else
  echo "[refs] 未探测到可用代理，按直连尝试（若失败请设 PROXY=http://host:port）"
fi

MODE="${1:-}"

status_line() {
  local name="$1" repo="$2" ref="$3"
  local d; d="$(cache_dir "$name" "$ref")"
  if [[ -f "$d/.fetch-ok" ]]; then
    echo "  ✅ $name@$ref  已缓存 ($d)"
  else
    echo "  ⬜ $name@$ref  未缓存（跑本脚本拉取）"
  fi
}

if [[ "$MODE" == "--list" ]]; then
  echo "[refs] 参考项目缓存状态（.cache/ 不入库，换机器需重拉）:"
  for entry in "${REFS[@]}"; do
    IFS='|' read -r name repo ref sub <<< "$entry"
    status_line "$name" "$repo" "$ref"
  done
  exit 0
fi

echo "[refs] 拉取参考项目源码（首次较慢；已缓存会直接 HIT）"
fail=0
for entry in "${REFS[@]}"; do
  IFS='|' read -r name repo ref sub <<< "$entry"
  echo "[refs] -- $name@$ref"
  # cache_fetch.sh 的 `sed` 取名在 macOS/BSD sed 上会报一行
  # "RE error: repetition-operator operand invalid"，但**不影响克隆** ——
  # 它只在推导默认缓存名时失败；这里显式给 --name 规避。
  if ! bash "$CACHE_FETCH" --repo "$repo" --ref "$ref" --subdir "$sub" --name "$name"; then
    echo "[refs] !! $name 拉取失败" >&2
    fail=1
  fi
done

echo
echo "[refs] 结果:"
for entry in "${REFS[@]}"; do
  IFS='|' read -r name repo ref sub <<< "$entry"
  status_line "$name" "$repo" "$ref"
done

cat <<'NOTE'

[refs] 注意事项（换机器 / 换网络时）
  1. 这些源码**不入库**（.cache/ 在 .gitignore 里），新电脑必须重跑本脚本。
     docs/opencode-parity.md 里的文件路径都指向 .cache/…，不重拉就查不到出处。
  2. github 直连可能失败（HTTP2 framing / 连接超时）。设代理：
       PROXY=http://127.0.0.1:7897 bash scripts/fetch-refs.sh
  3. MiMo Code **没有可用源码**：npm 包 @mimo-ai/cli 里是 99MB 编译产物
     (`bin/.mimocode`)，packages/ 目录不存在。要对照只能：
       - 装：npm i -g @mimo-ai/cli   （本机已有 /Users/kags/.npm-global/bin/mimo）
       - 然后用 pty 抓帧比对观感（见 PROJECT_MEMORY 4.51）
     不要试图去 clone XiaomiMiMo/MiMo-Code 找 TUI 源码 —— 那是另一个仓，
     与 npm 包不对应。
  4. opencode 的分支是 `dev`（不是 main）；只稀疏拉 `packages/tui`
     以省时间体积。要看别处再加 --subdir。

NOTE

exit $fail
