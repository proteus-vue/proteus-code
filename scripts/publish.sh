#!/usr/bin/env bash
#
# 按依赖顺序发布全部 crate 到 crates.io。
#
# 为什么需要这个脚本：crates.io 上每个 crate 的**依赖必须已经存在**。
# 直接 `cargo publish --workspace` 不存在，手动逐个发又极易顺序错乱
# （发了 neo-core 才发现 neo-protocol 还没发）。本脚本用 cargo metadata
# 算拓扑序，逐个发布，并在每个之后**有条件地等待**索引传播 —— 不盲等。
#
# 用法：
#   bash scripts/publish.sh --dry-run      # 演练：只打包，不真发（推荐先跑）
#   bash scripts/publish.sh                # 真发布（需要 CARGO_REGISTRY_TOKEN）
#
# 幂等：已发布过的 crate@version 会被跳过（查索引），可安全重跑。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

DRY=0
[ "${1:-}" = "--dry-run" ] && DRY=1
[ "${1:-}" = "--help" ] && { sed -n '2,20p' "$0" | sed 's/^# \?//'; exit 0; }

say() { printf '%s\n' "$*" >&2; }

# ── 计算拓扑序（依赖先发）──────────────────────────────────────────────
order="$(python3 - <<'PY'
import json, subprocess, sys
meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version", "1", "--no-deps"], text=True))
ws = set(meta["workspace_members"])
pkgs = {p["id"]: p for p in meta["packages"]}

# 只看**正常依赖**：dev-dependencies 不参与发布顺序。
# 例：neo-core 的 dev-dep 指向 neo-capability，而 neo-capability 正常依赖
# neo-core —— 把 dev 也算上会假报"成环"，实际发布没有环。
deps = {p["id"]: {d["name"] for d in p["dependencies"] if d.get("kind") is None}
        for p in meta["packages"]}
name2id = {p["name"]: i for i, p in pkgs.items() if i in ws}

out, done, onstack = [], set(), set()
def visit(pid):
    if pid in done: return
    if pid in onstack:
        sys.exit(f"内部依赖成环（仅正常依赖）：{pkgs[pid]['name']}")  # 架构守卫保证不会发生
    onstack.add(pid)
    for dep in sorted(deps[pid]):
        j = name2id.get(dep)
        if j: visit(j)
    onstack.discard(pid)
    done.add(pid)
    out.append(pkgs[pid]["name"])

for pid in sorted(ws, key=lambda i: pkgs[i]["name"]):
    visit(pid)
print("\n".join(out))
PY
)"
total=$(printf '%s\n' "$order" | wc -l | tr -d ' ')
say "拓扑序（$total 个 crate，依赖在前）："
printf '%s\n' "$order" | sed 's/^/  /' >&2

# ── 索引查询（判断 crate@version 是否已可用）───────────────────────────
idx_path() { # crates.io sparse index 路径规则
  local n="$1"
  case ${#n} in
    1) echo "1/$n" ;;
    2) echo "2/$n" ;;
    3) echo "3/${n:0:1}/$n" ;;
    *) echo "${n:0:2}/${n:2:2}/$n" ;;
  esac
}
already_published() { # $1=name
  local body
  body="$(curl -fsSL --connect-timeout 5 --max-time 30 "https://index.crates.io/$(idx_path "$1")" 2>/dev/null)" || return 2
  printf '%s' "$body" | grep -q "\"vers\":\"0.1.0\"" && return 0 || return 1
}
wait_index() { # $1=name  —— 用项目规范的 wait_for.sh（条件探测 + 总超时），不盲等
  local n="$1" script="$ROOT/docs/ai-efficiency-rules/scripts/wait_for.sh"
  if [ -f "$script" ]; then
    bash "$script" --cmd \
      "curl -fsSL --connect-timeout 5 --max-time 30 'https://index.crates.io/$(idx_path "$n")' | grep -q '\"vers\":\"0.1.0\"'" \
      --timeout 180 --interval 5 --max-interval 20 \
      && say "  ✓ 索引已收录 $n@0.1.0" \
      || say "  ⚠️  $n 在 180s 内未出现在索引 —— 后续 crate 可能因找不到它而失败"
  else
    say "  ⚠️  未找到 wait_for.sh，跳过索引等待（后续 crate 可能失败）"
  fi
}

# ── 逐个发布 ───────────────────────────────────────────────────────────
i=0
while IFS= read -r crate; do
  [ -z "$crate" ] && continue
  i=$((i+1))
  say ""
  say "[$i/$total] $crate"

  if already_published "$crate"; then
    say "  ⏭  已发布 0.1.0，跳过（幂等）"
    continue
  fi

  if [ "$DRY" -eq 1 ]; then
    cargo publish -p "$crate" --locked --dry-run --allow-dirty --no-verify
    continue
  fi

  cargo publish -p "$crate" --locked
  wait_index "$crate"
done <<< "$order"

say ""
if [ "$DRY" -eq 1 ]; then
  say "✅ 演练完成（未真正上传）。去掉 --dry-run 即真发布。"
else
  say "✅ 全部发布完成。验证：cargo install neo-code-cli --locked"
fi
