# AGENTS.md

给编码 Agent 的硬约束。开工前必读，违反任一「禁止」项的 PR 一律拒绝合入。

## 项目是什么

Rust + GPUI 原生桌面应用。分层：

```
<ns>-app      L4 应用组装
<ns>-core     与 UI 无关的业务核心（可 CLI 化、可无头测试）
<ns>-ui       L3 设计系统 + 带样式组件      ┐ 未来开源：组件库
<ns>-behavior L2 无样式行为层               │
<ns>-render   L1 渲染抽象 + GpuiBackend     ┘ 未来开源：渲染引擎
<ns>-kit      L0 门面层（唯一允许依赖 GPUI 的地方）
```

依赖方向**只能向下**。严禁反向，严禁跨层。

## 禁止（硬约束）

1. **禁止**在 `<ns>-kit` 之外的任何 `Cargo.toml` 声明 `gpui` / `gpui_platform` / `gpui-component`。
   → 原因：GPUI 类型身份分裂会让 Cargo 编译出两份引擎，报 `expected gpui::WindowContext, found gpui::WindowContext`。

2. **禁止**在 `crates/<ns>-behavior/src`、`crates/<ns>-ui/src` 中出现 `use gpui::`。
   → 渲染相关一律走 `<ns>-render` 的 `RenderBackend` trait。这是未来能开源渲染引擎的唯一保证。

3. **禁止**在 `<ns>-ui` 中出现业务名词。组件只认识 `Item` / `Row` / `Entry` / `Cell`。

4. **禁止**硬编码颜色与间距。颜色一律 `cx.theme()`，间距一律 design token。

5. **禁止**硬编码组件尺寸。只用 `.xsmall()` / `.small()` / `.medium()` / `.large()`。

6. **禁止**在 `<ns>-core` 中依赖任何 GUI crate。它必须 `cargo test -p <ns>-core` 无头通过。

7. **禁止**给 IME 相关测试加 `#[ignore]`。

8. **禁止**在升级 GPUI 版本的 PR 中夹带功能改动。升级 PR 只做升级。

## 必须

- 无状态组件 → `impl RenderOnce`；有状态逻辑 → `Entity<XxxState>` + `cx.subscribe_in`。
- 每个新组件：配 story 条目 + 截图基线 + 至少一个交互测试。
- 每个窗口最外层必须是 `Root::new(view, window, cx)`；`gpui_component::init(cx)` 必须在入口最先调用。
- 输入类组件必须有 IME 回归用例（preedit 删除后重输不 panic、候选框跟随光标）。
- 提交前跑：`cargo xtask verify-pins && cargo xtask verify-layering && cargo clippy -- -D warnings`。

## 常用命令

```bash
cargo xtask verify-pins         # 无 GPUI 版本分裂
cargo xtask verify-layering     # L2/L3 无 use gpui::
cargo xtask verify-assets       # 资源完整
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check
cargo xtask story               # 组件 gallery
cargo xtask screenshot-baseline # 更新截图基线
```

## 新写一个组件时的标准动作

1. 先确认 GPUI Kit 里**已有**——不要重造轮子（60+ 组件已存在，Apache-2.0，生产验证）。
2. 已有的 → 只做品牌化（token + 主题），不 fork 源码。
3. 确实没有的 → 按三段式写：`XxxState` / `XxxEvent` / `Xxx`（`RenderOnce`）。
4. 自绘类组件 → 走 `<ns>-render` 的 `RenderBackend`，不直接调 GPUI 绘制 API。
5. 补 story + 截图基线 + IME/键盘测试。
6. 过 DoD 九宫格（见 `docs/01-组件库规格.md` 第 3 节）。

## 升级 GPUI 版本时的标准动作

1. 只改 `<ns>-kit` 的 pin，其他 crate 不动。
2. 跑 `cargo tree -i gpui-ce --duplicates`，有输出说明分裂了，先修。
3. 跑截图基线比对，逐条 review 视觉差异。
4. 跑 IME 回归用例。
5. 单独发 PR，标题 `[deps] bump gpui-ce to X`，描述附基线比对结果。

## 拿不准时

- 代码放哪一层拿不准 → 放 L2（行为层）。表现错误好改，层次错误难救。
- 是否该走渲染缝拿不准 → 常规组件不走，自绘表面走。
- 是否该自研组件拿不准 → 先用 GPUI Kit 现成的，跑不通再说。

## 参考

- 架构决策：`docs/00-架构总纲.md`
- 组件 DoD 与清单：`docs/01-组件库规格.md`
- 阶段计划：`docs/02-落地路线图.md`
- GPUI Kit 文档：<https://gpui-kit.com/docs>
- GPUI-CE：<https://gpui-ce.github.io>
