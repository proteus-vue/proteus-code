# neo-ui-kit

GPUI 生态的门面：全工程**唯一**声明 GPUI 系列依赖的地方。

## 为什么需要这一层

GPUI 生态有一个很难诊断的坑：**类型身份分裂**。如果工程里有两个 crate 各自
声明了不同来源或版本的 gpui，Cargo 会编译两份引擎，然后报出这样的错误：

```text
expected gpui::WindowContext, found gpui::WindowContext
```

两个类型名字完全一样，却不是同一个类型。读错误信息几乎定位不到原因，
因为问题在依赖图里，不在出错的那行代码上。

本 crate 的角色是**把 gpui 系列依赖收敛到一处**，其余所有 crate 通过它取用：

```rust,ignore
// 其它 crate 这样取用（不要在它们的 Cargo.toml 里声明 gpui）
use neo_ui_kit::gpui::{div, px, IntoElement};
use neo_ui_kit::component::{Button, Root};
```

## 为什么 pin 的是 `gpui-kit`

`gpui-kit` 把 gpui、平台后端、基础层、组件库、资源层一并带齐，且版本自洽。
只声明它一个依赖，就能同时满足"用得上全部四层"与"不会出现两份引擎"。

直接声明上游 `gpui` + 组件库的组合则容易踩到版本不匹配 —— 组件库的版本约束
未必与你挑的 gpui 版本兼容，而症状就是开头那个难以定位的类型错误。

本 crate 也顺带处理了一件事：`gpui_platform` 未单独发布，所以直接用
`gpui::Application::new()` 会要求调用方自己提供平台后端。这里提供了一个
已挂好后端的入口：

```rust,ignore
// 这段**不能**当文档测试跑：`run` 会启动图形事件循环并阻塞。
// 标 `ignore` 是刻意的 —— 它是一份"怎么接线"的说明，不是可执行样例。
use neo_ui_kit::{application, init};

fn main() {
    application()
        .with_assets(neo_ui_kit::assets::Assets)
        .run(move |cx| {
            init(cx);  // 装主题与各组件全局状态，**建窗口之前**调用一次
            // ... 在这里开窗口
        });
}
```

## 四层出口

| 路径 | 内容 |
|---|---|
| `neo_ui_kit::gpui` | 引擎本体（元素、事件、窗口、几何） |
| `neo_ui_kit::base` | 基础层（主题系统、通用组件基础件、文本编辑引擎） |
| `neo_ui_kit::component` | 组件库（按钮、输入、列表、表格、对话框…） |
| `neo_ui_kit::platform` | 平台后端 |
| `neo_ui_kit::assets` | 图标资源 |
| `neo_ui_kit::kit_test` | 测试基建：无头窗口 + 交互派发（需 `test-support` feature） |

## 测试基建

UI 交互（点击、键盘、输入）在无头窗口里可测，不必启动真实窗口：

```toml
[dev-dependencies]
neo-ui-kit = { version = "...", features = ["test-support"] }
```

该 feature **默认关闭**，因为它会打开各层自身的 test-support，编进发布产物
是纯粹的浪费。需要它的只有测试：

```rust,ignore
use neo_ui_kit::kit_test::TestWindowExt as _;

// 在 #[gpui::test] 里：
//   window.click("my-button", cx);
//   window.press("escape", cx);
//   window.input("文本", cx);
//   window.render_frame(cx);
//   let snap = window.find("my-input");
```

注意 `#[gpui::test]` 宏展开时引用 `gpui::` 路径，所以使用方需要
`use neo_ui_kit::gpui as gpui;` 提供一个别名 —— 这是宏的路径硬编码所致，
**不是**要求你在 Cargo.toml 里声明 gpui 依赖（那会破坏本 crate 存在的意义）。

## 诚实边界

- 本 crate **不隐藏** gpui —— 它 re-export 全部四层，因此使用方会直接面对
  gpui 的 API 与版本节奏。这是有意的：包装一套"完全自定义的 UI 抽象"
  会立刻失去组件库的性能特性，且维护成本极高。
- 它**不是**兼容层：不试图对不同版本的 gpui 提供统一接口。
- 版本 pin 在依赖方工程的 `[workspace.dependencies]` 里（本项目只有一个
  crate 引用它，所以只有一处版本号）。

## 许可

MIT，见 [LICENSE](LICENSE)。
