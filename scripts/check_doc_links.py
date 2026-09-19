#!/usr/bin/env python3
"""
文档站链接自检 —— 索引页里每个相对链接都必须**指得到东西**。

用法：python3 scripts/check_doc_links.py dist/docs

# 为什么需要它（这是本轮实测加上的，不是预防性编程）

第一版索引页写的是 `../neo-plan/README.md` —— 那是**仓库内**的相对路径。
一旦产物被单独托管（或只是换个目录打开），那些链接**全部失效**。
实测：40 个链接里 **10 个指不到东西**。

而且这个错误**不会以任何显眼方式暴露**：
- `cargo doc` 绿（它只管 API 文档内部链接）；
- 生成脚本跑完也报"成功"；
- 只有**点下去**才发现 404。

所以"站点生成成功"与"站点是好的"是两件事 —— 这条脚本补的是后者。

# 它检查什么、不检查什么

**检查**：索引页（`index.html`）里的每个相对 `href`，解析后必须存在于产物目录。
**不检查**：
- API 文档内部的链接（那些由 `cargo doc` 的 `broken_intra_doc_links` 会把关，
  且 `verify.sh` 已把 doc 零 warning 收进门禁）；
- 外链（`http(s)://`）—— 不联网，也不该因为对方站点抖动而卡住本地构建；
- 锚点（`#fragment`）—— 需要解析 HTML 才能验证，收益不抵复杂度。

**这条边界是刻意的**：一个"静默不生效"的检查比没有更糟（本仓对守卫的一贯要求）。
"""
import os
import re
import sys
import urllib.parse


def main() -> int:
    if len(sys.argv) < 2:
        print("用法：python3 scripts/check_doc_links.py <站点目录>")
        return 2
    site = sys.argv[1]
    idx = os.path.join(site, "index.html")
    if not os.path.isfile(idx):
        print(f"❌ 找不到索引页：{idx}")
        return 1

    with open(idx, encoding="utf-8") as f:
        html = f.read()

    links = re.findall(r'href="([^"]+)"', html)
    checked = 0
    bad = []
    for link in links:
        if link.startswith(("http://", "https://", "#", "mailto:")):
            continue
        path = urllib.parse.unquote(link.split("#")[0])
        # 相对站点根解析（索引页在站点根）
        target = os.path.normpath(os.path.join(site, path))
        checked += 1
        if not os.path.exists(target):
            bad.append(link)

    if bad:
        print(f"❌ 文档站有 {len(bad)} 个坏链接（共核对 {checked} 个）：")
        for b in bad:
            print(f"     {b}")
        print()
        print("  常见原因：链接指向了**产物目录之外**（如仓库上级的 ../xxx）。")
        print("  修法：把手写文档复制进站点（见 build-docs.sh 的 repo/ 那一段），")
        print("  并把索引页的链接改成站点内路径。")
        return 1

    print(f"  ✅ 文档站链接自检通过（{checked} 个相对链接全部指向存在的文件）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
