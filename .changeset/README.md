# .changeset —— 变更碎片

这个目录放**待发布的改动说明**。每个改动一条碎片，由
`scripts/changeset.py` 聚合出版本号与 CHANGELOG。

## 怎么加一条

```bash
python3 scripts/changeset.py new --bump minor --note "新增 npm 分发"
```

`--bump` 决定这次改动推进哪个版本段：

| 级别 | 什么时候用 |
|---|---|
| `patch` | 修 bug、改文档与内部实现，行为对外无变化 |
| `minor` | 新增能力（向后兼容） |
| `major` | 破坏性变更（改协议、改 CLI 参数语义、不兼容的行为变更） |

> 本仓库是**单一版本工作区**：23 个 crate 共享 `[workspace.package] version`，
> 所以碎片只写级别、不写包名。若将来某个 crate 需要独立版本，这里再改成
> changesets 那种 `"包名": 级别` 的写法。

## 文件长什么样

`20260914120000-add-npm.md`：

```markdown
---
bump: minor
---

新增 npm 分发：`npm install -g neo-code`。
```

正文会**原样**进 `CHANGELOG.md`，所以写给用户看，不要写"修改了 xxx 函数"。

## 之后会发生什么

1. 你把这个文件随改动一起提交进 PR（CI 会检查有没有带）。
2. 合并到 `main` 后，机器人开一个 **Version PR**：把版本推进、汇总 CHANGELOG、
   删掉已消费的碎片。
3. 你 review 后合并那个 PR —— **这一步才真正发布**（打 tag → 构建三平台 →
   GitHub Release → npm）。

完整流程与排错见 [`docs/RELEASE.md`](../docs/RELEASE.md)。

## 豁免

纯重构、CI、文档类改动不需要 changeset。给 PR 打上 `no-changeset` 标签即可
跳过 CI 检查。
