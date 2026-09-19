#!/usr/bin/env python3
"""
最小 markdown → HTML 转换器 —— 给文档站用。

    python3 scripts/md_to_html.py <输入.md> <输出.html> [--title 标题]

# 为什么自己写（而不是引一个 markdown 库）

与文档站选 `cargo doc` 而非 mdbook 是同一个判断：**不为一点需要加工具**。
本仓是 Rust 项目、没有 Python 依赖清单，引 `markdown`/`markdown2` 意味着
文档站多一个 pip 前置；而这里要转换的是**我们自己的文档**，用到的语法是
一个已知的有限子集。

# 它支持什么（按本仓文档的实际用法裁剪）

- ATX 标题 `#`..`######`
- 围栏代码块 ``` ``` ```（带语言标注 → `class="language-x"`，不引高亮器）
- GFM 表格（含对齐分隔行）
- 无序/有序列表（含嵌套，靠缩进判断）
- 任务列表 `- [ ]` / `- [x]`
- 引用块 `> `
- 分隔线 `---` / `***`
- 行内：`code`、**粗体**、*斜体*、~~删除线~~、[链接](url)

**不支持**（如实列出来，而不是装作支持）：
- 引用式链接 `[x][1]`、脚注、定义列表、内嵌 HTML（会被转义显示）
- 表格单元格内的块级元素
- 代码高亮（语法着色）—— 只给 `<pre><code>`，靠浏览器等宽字体

# 两条实现上的硬要求

1. **必须先转义 HTML 再套格式**：本仓文档里大量出现 `<name>`、`<800ms`、
   `<dir>` 这类写法（是**文字**不是标签）。先转义再处理行内标记，
   才能保证它们显示为字面量 —— 否则 `<name>` 会被浏览器当标签吃掉。
2. **代码块与行内 code 里不做任何标记替换**：`**` 在代码里就是两个星号。
   所以先把 code 段抽出来占位，处理完其余部分再放回。
"""
import html
import re
import sys


def esc(text: str) -> str:
    """转义为 HTML 文本（`&` `<` `>` `"` 四个）。"""
    return html.escape(text, quote=False)


def inline(text: str) -> str:
    """行内标记 → HTML。**输入必须是已转义的文本。**"""
    # ① 先把行内 code 抽成占位符：它内部不做任何替换。
    codes: list[str] = []

    def stash(m: re.Match) -> str:
        codes.append(m.group(1))
        return f"\x00CODE{len(codes) - 1}\x00"

    # 单反引号（本仓文档里的行内 code 都是单反引号；双反引号极少见，一并支持）
    text = re.sub(r"``(.+?)``", stash, text, flags=re.S)
    text = re.sub(r"`([^`]+)`", stash, text, flags=re.S)

    # ② 链接：`[文字](url)`。url 里可能有空格（本仓有含空格的路径），所以不强拆。
    text = re.sub(
        r"\[([^\]]+)\]\(([^)\s]+)\)",
        lambda m: f'<a href="{m.group(2)}">{m.group(1)}</a>',
        text,
    )
    # ③ 强调。顺序要紧：`**` 先于 `*`，否则 `**x**` 会被拆成两个 `*`。
    text = re.sub(r"\*\*(.+?)\*\*", r"<strong>\1</strong>", text)
    text = re.sub(r"(?<!\*)\*([^*\n]+?)\*(?!\*)", r"<em>\1</em>", text)
    text = re.sub(r"~~(.+?)~~", r"<del>\1</del>", text)

    # ④ 放回 code（内容已转义过，直接包 <code>）
    for i, c in enumerate(codes):
        text = text.replace(f"\x00CODE{i}\x00", f"<code>{c}</code>")
    return text


def split_table_row(line: str) -> list[str]:
    """拆一行表格。首尾的 `|` 要先去掉。"""
    s = line.strip()
    if s.startswith("|"):
        s = s[1:]
    if s.endswith("|"):
        s = s[:-1]
    return [c.strip() for c in s.split("|")]


def is_table_sep(line: str) -> bool:
    """`|---|:--:|` 这种分隔行。"""
    s = line.strip()
    if not s.startswith("|"):
        return False
    cells = split_table_row(s)
    return bool(cells) and all(re.fullmatch(r":?-{1,}:?", c) for c in cells)


def convert(md: str) -> str:
    lines = md.replace("\r\n", "\n").split("\n")
    out: list[str] = []
    i = 0
    n = len(lines)

    # 列表栈：每项是 ("ul"|"ol", 缩进层级)
    list_stack: list[tuple[str, int]] = []
    para: list[str] = []

    def close_lists(to_depth: int = 0) -> None:
        while len(list_stack) > to_depth:
            kind, _ = list_stack.pop()
            out.append(f"</{kind}>")

    def flush_para() -> None:
        if para:
            joined = " ".join(x.strip() for x in para if x.strip())
            if joined:
                out.append(f"<p>{inline(joined)}</p>")
            para.clear()

    def indent_of(line: str) -> int:
        return len(line) - len(line.lstrip(" "))

    while i < n:
        line = lines[i]
        stripped = line.strip()

        # ── 围栏代码块 ──
        if stripped.startswith("```") or stripped.startswith("~~~"):
            flush_para()
            close_lists()
            fence = stripped[:3]
            lang = stripped[3:].strip()
            i += 1
            buf: list[str] = []
            while i < n and not lines[i].strip().startswith(fence):
                buf.append(lines[i])
                i += 1
            i += 1  # 跳过结束围栏
            cls = f' class="language-{esc(lang)}"' if lang else ""
            body = esc("\n".join(buf))
            out.append(f"<pre><code{cls}>{body}</code></pre>")
            continue

        # ── 空行 ──
        if not stripped:
            flush_para()
            # 空行只在**同级列表项之间**保持列表打开；缩进更浅则收口
            if list_stack:
                # 看看后面还有没有同层或更深层的列表项
                j, deeper = i + 1, False
                while j < n and lines[j].strip():
                    if re.match(r"^\s*([-*+]|\d+[.)])\s", lines[j]):
                        deeper = True
                    break
                if not deeper:
                    close_lists()
            i += 1
            continue

        # ── 分隔线 ──
        if re.fullmatch(r"\s*([-*_])\s*(\1\s*){2,}", line) and "- [" not in line:
            flush_para()
            close_lists()
            out.append("<hr>")
            i += 1
            continue

        # ── 标题 ──
        m = re.match(r"^(#{1,6})\s+(.*)$", stripped)
        if m:
            flush_para()
            close_lists()
            level = len(m.group(1))
            out.append(f"<h{level}>{inline(esc(m.group(2).strip()))}</h{level}>")
            i += 1
            continue

        # ── 表格 ──
        if stripped.startswith("|") and i + 1 < n and is_table_sep(lines[i + 1]):
            flush_para()
            close_lists()
            header = split_table_row(line)
            aligns = split_table_row(lines[i + 1])
            i += 2
            rows: list[list[str]] = []
            while i < n and lines[i].strip().startswith("|"):
                rows.append(split_table_row(lines[i]))
                i += 1

            def align_attr(col: int) -> str:
                if col >= len(aligns):
                    return ""
                a = aligns[col]
                if a.startswith(":") and a.endswith(":"):
                    return ' style="text-align:center"'
                if a.endswith(":"):
                    return ' style="text-align:right"'
                return ""

            out.append("<table>")
            out.append("<thead><tr>")
            for c, cell in enumerate(header):
                out.append(f"<th{align_attr(c)}>{inline(esc(cell))}</th>")
            out.append("</tr></thead><tbody>")
            for r in rows:
                out.append("<tr>")
                # 单元格数与表头不一致时按表头补齐/截断（markdown 允许多余空格）
                for c in range(len(header)):
                    cell = r[c] if c < len(r) else ""
                    out.append(f"<td{align_attr(c)}>{inline(esc(cell))}</td>")
                out.append("</tr>")
            out.append("</tbody></table>")
            continue

        # ── 引用块（连续行合并）──
        if stripped.startswith(">"):
            flush_para()
            close_lists()
            buf = []
            while i < n and lines[i].strip().startswith(">"):
                buf.append(lines[i].strip()[1:].strip())
                i += 1
            out.append(f"<blockquote>{inline(esc(' '.join(buf)))}</blockquote>")
            continue

        # ── 列表 ──
        m = re.match(r"^(\s*)([-*+]|\d+[.)])\s+(.*)$", line)
        if m:
            flush_para()
            indent = len(m.group(1))
            marker, body = m.group(2), m.group(3)
            kind = "ol" if marker[0].isdigit() else "ul"
            # 缩进 → 层级（每 2 空格算一层，本仓文档用 2 或 4）
            depth = indent // 2 + 1

            # 收口到当前层级
            while list_stack and list_stack[-1][1] >= depth:
                prev_kind, prev_depth = list_stack[-1]
                if prev_depth == depth and prev_kind == kind:
                    break
                close_lists(len(list_stack) - 1)
            if not list_stack or list_stack[-1][1] < depth:
                out.append(f"<{kind}>")
                list_stack.append((kind, depth))

            # 任务列表
            tm = re.match(r"^\[([ xX])\]\s+(.*)$", body)
            if tm:
                checked = " checked" if tm.group(1).lower() == "x" else ""
                body = (
                    f'<input type="checkbox" disabled{checked}> {inline(esc(tm.group(2)))}'
                )
            else:
                body = inline(esc(body))
            out.append(f"<li>{body}</li>")
            i += 1
            continue

        # ── 普通段落行（续行合并）──
        para.append(line)
        i += 1

    flush_para()
    close_lists()
    return "\n".join(out)


PAGE = """<!doctype html>
<html lang="zh-CN">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title}</title>
<style>
  /* 与索引页同一套配色约定：**显式成对**给前景/背景（只写 color-scheme
     会让浏览器在暗色系统下选浅色字压白底，几乎读不出来）。*/
  :root {{ color-scheme: light dark }}
  body {{
    font: 15px/1.7 -apple-system, "Segoe UI", "Noto Sans CJK SC", sans-serif;
    max-width: 54rem; margin: 3rem auto; padding: 0 1.25rem;
    color: #1a1a1a; background: #fff;
  }}
  @media (prefers-color-scheme: dark) {{
    body {{ color: #e6e6e6; background: #1b1b1b }}
    a {{ color: #a78bfa }}
    code {{ background: rgba(255,255,255,.12) }}
    th {{ background: rgba(255,255,255,.06) }}
    blockquote {{ border-color: rgba(255,255,255,.25); color: #b9b9b9 }}
  }}
  a {{ color: #6d4aff }}
  h1 {{ font-size: 1.7rem; margin-top: 0 }}
  h2 {{ font-size: 1.25rem; margin-top: 2.2rem; padding-top: .4rem }}
  h3 {{ font-size: 1.05rem; margin-top: 1.6rem }}
  code {{
    background: rgba(0,0,0,.07); padding: .1em .35em; border-radius: 4px;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: .92em;
  }}
  pre {{
    background: rgba(127,127,127,.12); padding: .8rem 1rem; border-radius: 6px;
    overflow-x: auto;
  }}
  pre code {{ background: none; padding: 0 }}
  table {{ border-collapse: collapse; margin: 1rem 0; width: 100%; font-size: .95em }}
  th, td {{ border: 1px solid rgba(127,127,127,.35); padding: .4rem .6rem; text-align: left }}
  th {{ background: rgba(0,0,0,.05) }}
  blockquote {{
    margin: 1rem 0; padding: .2rem 0 .2rem 1rem;
    border-left: 3px solid rgba(127,127,127,.5); color: #555;
  }}
  hr {{ border: none; border-top: 1px solid rgba(127,127,127,.35); margin: 2rem 0 }}
  li {{ margin: .25rem 0 }}
  .banner {{
    font-size: .85em; opacity: .7; margin-bottom: 2rem;
    padding-bottom: .6rem; border-bottom: 1px solid rgba(127,127,127,.3);
  }}
</style>
<div class="banner">由 <code>scripts/build-docs.sh</code> 从仓库内的 markdown 生成
（<a href="../index.html">← 回文档索引</a>）</div>
{body}
</html>
"""


def main() -> int:
    if len(sys.argv) < 3:
        print("用法：python3 scripts/md_to_html.py <输入.md> <输出.html> [--title 标题]")
        return 2
    src, dst = sys.argv[1], sys.argv[2]
    title = None
    if "--title" in sys.argv:
        title = sys.argv[sys.argv.index("--title") + 1]

    with open(src, encoding="utf-8") as f:
        md = f.read()

    if title is None:
        # 取第一个一级标题当页面标题；没有就用文件名
        m = re.search(r"^#\s+(.+)$", md, re.M)
        title = m.group(1).strip() if m else src.rsplit("/", 1)[-1]

    body = convert(md)
    # 站内 md 互链 → 指向同名 .html
    html_doc = PAGE.format(title=esc(title), body=body)
    html_doc = re.sub(r'href="([^"]+)\.md(#[^"]*)?"', r'href="\1.html\2"', html_doc)

    with open(dst, "w", encoding="utf-8") as f:
        f.write(html_doc)
    return 0


if __name__ == "__main__":
    sys.exit(main())
