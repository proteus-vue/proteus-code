#!/usr/bin/env bash
# 构建文档站 —— `cargo doc`（API 参考）+ 手写文档索引（prose）。
#
#   bash scripts/build-docs.sh              # 产物在 dist/docs/
#   bash scripts/build-docs.sh --no-deps    # 只建本仓 crate 的文档（快，CI 默认）
#
# # 为什么是 `cargo doc` 而不是 mdbook
#
# 方案 Phase 3 的原文是「文档站：`cargo doc` **或** mdbook」—— 两者都算达标。
# 选前者的理由：
#
# 1. **零新增工具**。mdbook 是一个要单独安装的外部二进制（本机就没有），
#    而本仓对"为一点需要引入一个新工具"一向克制（同样的判断在渲染缝的
#    自写光栅化器那里做过一次）。
# 2. **Rust 项目的文档主体本来就是 doc comment**。本仓 5 个可开源 crate 的
#    模块头注释写得很重（每条设计取舍都在那里），`cargo doc` 直接把它们
#    组织成可跳转的站点 —— 这是**不用维护第二份文档**的那条路。
# 3. **它对坏链接敏感**：`cargo doc` 报 broken intra-doc link，而 mdbook
#    对 markdown 里的相对链接也能报 —— 但 doc comment 里的 `[\`Foo\`]`
#    只有 rustdoc 能把关。本轮实测清掉了 16 条 doc warning（含 4 处坏链接），
#    那些**在 mdbook 路径下根本不会被发现**。
#
# # 手写文档怎么办
#
# `docs/` 下的 markdown（方案、parity、发布流程…）不搬进 `cargo doc` ——
# 它们不是 API 文档，硬塞进去会把 rustdoc 变成杂物间。做法是生成一个
# **索引页**列出它们（指向仓库内路径），让站点有一个能进的门口。
#
# # 诚实边界
#
# - **不部署**：产物落在 `dist/docs/`，上传/托管是发布动作（另说）。
# - **markdown 不渲染成 HTML**：索引页只**链接**到仓库里的 `.md`。把它们
#   渲染成 HTML 需要引入一个 markdown 渲染器（同上：不为这点加工具）。
# - `--no-deps` 是**默认**：只建本仓 31 个 crate 的文档，不建依赖树
#   （那要几分钟且与本项目无关）。
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

OUT="dist/docs"

# 工具链预检（与 verify.sh 同一套）：选错 cargo 会伪装成编译错误
. "$ROOT/scripts/lib-rust-toolchain.sh"
ensure_pinned_cargo || exit 1

echo "== 构建 API 文档（cargo doc --workspace --no-deps）=="
# `--no-deps` 默认开：建本仓 crate 即可。加 `--with-deps` 才建依赖树。
if [ "${1:-}" = "--with-deps" ]; then
  cargo doc --workspace || exit 1
else
  cargo doc --workspace --no-deps || exit 1
fi

echo "== 组装站点到 $OUT =="
rm -rf "$OUT"
mkdir -p "$OUT"
cp -R target/doc/. "$OUT/"

# 把**手写文档**也复制进站点（`repo/` 子目录），而不是链接到仓库上级。
#
# ⚠️ 为什么必须复制：第一版索引页写的是 `../neo-plan/README.md` —— 那是**仓库
# 内**的相对路径。产物一旦被单独托管（或只是换个目录打开），那些链接**全部失效**。
# 实测：40 个链接里 10 个指不到东西。**"能生成"不等于"站点里没有坏链接"** ——
# 所以下面复制完还要逐个核对（见脚本末尾）。
#
# 只复制文档类文件：`.md` + 少量结构化目录，不搬整个仓库（那会把产物从
# 几百 KB 撑成几十 MB，还带上源码与构建产物）。
#
# 用 python 走一遍比 shell 的 `find | xargs | cp` 可靠得多 —— 后者要处理
# 目录创建与中文路径，我第一版就是这么写的，结果 40 个链接里仍有 10 个指不到。
mkdir -p "$OUT/repo"
python3 - "$ROOT" "$OUT" <<'PYEOF'
import os, shutil, sys

root, out = sys.argv[1], sys.argv[2]
dest = os.path.join(out, "repo")

# 顶层项目文档
for f in ("README.md", "CONTRIBUTING.md", "PROJECT_MEMORY.md", "AGENTS.md",
          "THIRD-PARTY-LICENSES.md"):
    src = os.path.join(root, f)
    if os.path.isfile(src):
        shutil.copy2(src, dest)

# 文档目录：只拿 markdown（排除 __pycache__ / 构建残留）
for sub in ("docs/neo-plan", "docs/spi-first-methodology"):
    src_dir = os.path.join(root, sub)
    if not os.path.isdir(src_dir):
        continue
    for dirpath, dirnames, filenames in os.walk(src_dir):
        dirnames[:] = [d for d in dirnames if d != "__pycache__"]
        for fn in filenames:
            if not fn.endswith(".md"):
                continue
            s_path = os.path.join(dirpath, fn)
            rel = os.path.relpath(s_path, root)
            d_path = os.path.join(dest, rel)
            os.makedirs(os.path.dirname(d_path), exist_ok=True)
            shutil.copy2(s_path, d_path)

# 顶层 parity / 流程文档
docs_dir = os.path.join(root, "docs")
for fn in sorted(os.listdir(docs_dir)):
    if fn.endswith(".md"):
        os.makedirs(os.path.join(dest, "docs"), exist_ok=True)
        shutil.copy2(os.path.join(docs_dir, fn), os.path.join(dest, "docs", fn))
PYEOF

# 把站内的 markdown **渲染成 HTML**（`.md` 同时保留，便于"看源码"）。
#
# ⚠️ 这一步是本轮补上的：第一版只把 `.md` 复制进站点，于是索引页点进去是
# **原始 markdown** —— 表格、标题全不渲染，一片等宽文字。那等于没做文档站。
#
# 之所以先"没做"，是因为我当时把它记成了"诚实边界"（"不引 markdown 渲染器，
# 所以是源码形态"）—— 但**看到实际效果才明白那不是边界，是缺陷**。
# 这条教训值得记：把"没做"写成"边界"会让它看起来像有意为之，从而不再被修。
echo "== 渲染 markdown → HTML =="
converted=0
while IFS= read -r -d '' md; do
  python3 "$ROOT/scripts/md_to_html.py" "$md" "${md%.md}.html" || exit 1
  converted=$((converted + 1))
done < <(find "$OUT/repo" -name '*.md' -print0)
echo "   已渲染 ${converted} 个 markdown 页面"

# 生成索引页。
#
# 它不追求好看 —— 追求**能进得去、且不撒谎**：列出的每一项都必须真的存在
#（下面用 `test -f` 逐个核对），否则索引本身就是坏的。
CRATES=$(ls crates | sort)
{
  cat <<'HTML'
<!doctype html>
<html lang="zh-CN">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>NEO 文档</title>
<style>
  /* ⚠️ 颜色必须**显式成对**给（前景 + 背景），不能只写 `color-scheme`。
     #
     # 实测踩到：第一版只写了 `color-scheme: light dark`，于是浏览器在**暗色
     系统**下按暗色方案选字色（浅灰），而页面背景仍是默认白 ——
     **浅灰字压白底，几乎读不出来**（截图才看见，`curl` 与 domSnapshot 都是"正常"的）。*/
  :root { color-scheme: light dark; }
  body {
    font: 15px/1.6 -apple-system, "Segoe UI", "Noto Sans CJK SC", sans-serif;
    max-width: 54rem; margin: 3rem auto; padding: 0 1.25rem;
    /* 成对显式给：亮色方案下的字色与底色 */
    color: #1a1a1a; background: #ffffff;
  }
  @media (prefers-color-scheme: dark) {
    body { color: #e6e6e6; background: #1b1b1b; }
    a { color: #a78bfa; }
    code { background: rgba(255,255,255,.12); }
  }
  a { color: #6d4aff; }
  h1 { font-size: 1.6rem } h2 { font-size: 1.1rem; margin-top: 2rem }
  ul { padding-left: 1.2rem } li { margin: .25rem 0 }
  code { background: rgba(0,0,0,.07); padding: .1em .35em; border-radius: 4px }
  .note { opacity: .75; font-size: .9em }
</style>
<h1>NEO 文档</h1>
<p class="note">本页由 <code>scripts/build-docs.sh</code> 生成。</p>
<h2>API 参考（各 crate）</h2>
<ul>
HTML
  for c in $CRATES; do
    # 站点目录名是 crate 名把 `-` 换成 `_`
    site="${c//-/_}"
    if [ -f "$OUT/$site/index.html" ]; then
      printf '  <li><a href="%s/index.html"><code>%s</code></a></li>\n' "$site" "$c"
    fi
  done
  cat <<'HTML'
</ul>
<h2>可开源集（Apache-2.0）</h2>
<ul>
  <li><code>neo-text</code> — 宿主中立的文本语义（色调 / 宽度 / Markdown / 高亮）</li>
  <li><code>neo-ui-kit</code> — L1 UI 门面（唯一 pin GPUI 的地方）</li>
  <li><code>neo-ui-render</code> — L2 渲染缝（含两个后端 + 软件光栅化器）</li>
  <li><code>neo-ui-behavior</code> — L3 UI 行为层（纯逻辑，不依赖 GPUI）</li>
  <li><code>neo-ui</code> — L4 设计系统（品牌主题 + 自研组件）</li>
</ul>
<p class="note">这五个目录可整体复制出去独立编译（由 <code>check_extractable.py</code> 每次门禁验证）。</p>
<h2>设计文档（仓库内 markdown）</h2>
<ul>
  <li><a href="repo/docs/neo-plan/README.html">方案总纲</a> — 六层架构、模块规格、里程碑、验证套件</li>
  <li><a href="repo/docs/neo-plan/02-架构设计/Proteus方法论-语义核心与后端SPI.html">架构设计</a> — 方法论、事件协议、状态机、正交双轴</li>
  <li><a href="repo/docs/neo-plan/04-落地计划/里程碑与验收标准.html">里程碑与验收标准</a></li>
  <li><a href="repo/docs/neo-plan/05-验证/验证矩阵.html">验证套件</a></li>
  <li><a href="repo/docs/spi-first-methodology/README.html">SPI-First 方法论</a> — 跨 16 次生产实践泛化</li>
  <li><a href="repo/docs/desktop-parity.html">桌面 parity</a> — 逐项对照表</li>
  <li><a href="repo/docs/RELEASE.html">发布流程</a></li>
</ul>
<h2>项目文档</h2>
<ul>
  <li><a href="repo/README.html">README</a></li>
  <li><a href="repo/CONTRIBUTING.html">贡献指南</a></li>
  <li><a href="repo/PROJECT_MEMORY.html">项目记忆</a>（为什么这样做、踩过什么坑）</li>
</ul>
</body>
</html>
HTML
} > "$OUT/index.html"

# 链接自检：索引页里每个相对链接都必须**指得到东西**。
#
# 这条是本轮实测加上的：第一版索引页 40 个链接里有 10 个指不到（都指向仓库
# 上级，单独托管即失效）。"能生成"不等于"站点是好的" —— 而坏链接在站点里
# 只有点下去才发现。
python3 "$ROOT/scripts/check_doc_links.py" "$OUT" || exit 1

count=$(find "$OUT" -name 'index.html' | wc -l | tr -d ' ')
echo "✅ 文档站已生成：${OUT}（${count} 个页面，链接已逐个核对）"
echo "   本地查看：open ${OUT}/index.html"
