---
bump: patch
---

CI 失败摘要：把内联 grep 换成可回放的 `scripts/ci-failure-digest.sh`

上一版在 workflow 里用 `grep -i 'error|failed'` 发注解，两个硬约束叠加导致
**真因读不到**：GitHub 每个 step 只保留前 10 条 error 注解，而这条 grep 会把
`Compiling thiserror`、`test result: ok. 0 failed` 两类噪声排在最前，配额被
吃光，真正的 `FAILED`/`panic` 行一条都没发出去。新脚本按价值排序（失败用例名 →
panic 上下文 → 编译错误 → 链接期失败），error/warning 各 10 条，模式行首锚定
且大小写敏感；并带 `--selftest` 自检，由 `verify.sh` 第 6 段执行 —— 已实测把
模式改回旧的 `-i` 写法会立刻变红。
