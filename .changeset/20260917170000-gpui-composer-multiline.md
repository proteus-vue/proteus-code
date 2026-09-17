---
bump: minor
---

gpui composer 改为多行：Enter 提交、Shift+Enter 换行（并修掉回车留下换行的缺陷）

缺口表里"gpui 宿主无多行输入（`Shift+Enter` 什么都不做）"已补：输入框从单行
`InputState` 换成 `TextareaState`，`submit_on_enter(true)` + `auto_grow(1, 6)`。

**顺带抓到一个真缺陷**（端到端实测，读组件文档看不出来）：平台的 Enter 会被
当作文本再送一遍（macOS 平台层给 Enter 的 `key_char` 就是 `"\n"`），而组件在
"提交"分支里 `cx.propagate()` —— 于是**回车既提交、又留下一个换行**，输入框
看着没清干净。修法是在 `composer_input` 里 `stop_propagation`，提交事件在那之前
已派发，故不受影响。

真机证据：`type` 两行 + `shift+return` 后，无障碍树里 `任务输入 = first\nsecond`；
按 `return` 后输入框清空且无残留换行，会话日志为
`{"begin_turn":{"text":"first\nsecond"}}` + `user_submitted`。

4 条无头回归用例（`tests/composer_multiline.rs`）钉住多行语义；牙齿已验证 ——
去掉 `submit_on_enter` 或去掉 `stop_propagation`，对应用例立刻变红。
