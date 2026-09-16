# neo-ui-render

渲染缝：把"要画什么"（中立场景描述）与"用什么画"（具体图形后端）分开。

## 这个 crate 是什么，不是什么

它是**一条缝**，不是渲染引擎：

```text
调用方
  │  构造 Scene（中立：颜色 + 几何 + 绘制指令）
  ▼
RenderBackend（trait）
  │
  ▼
具体后端（目前只有 GpuiBackend 一个实现）
```

**目前只有一个实现。** 这一点必须说清楚 —— 只有一个实现的抽象是**尚未被证明的
设计**：它对不对，要等第二个实现来检验。本 crate 不假装自己已经是成熟的
可替换抽象。

## 它是给谁用的

**只服务"自绘表面"** —— 图表、diff 视图、波形、拓扑图这类你本来就要自己写
绘制代码的地方。

常规组件（按钮、输入框、列表、表格）**不应该走这条缝**，直接用组件库即可。
原因是具体的：走缝意味着放弃组件库的 element diff、脏区剔除与字形缓存，
那是纯粹的性能倒退，而抽象成本却省不下来（缝本身要维护）。

```text
常规组件   → 直接用组件库，享受全部性能特性
自绘表面   → 走这条缝，抽象成本≈0（本来就在自己写绘制）
```

## 用法

### 构造一个场景

```rust
use neo_ui_render::{Color, Op, Rect, Scene};

let mut scene = Scene::new();
scene.push(Op::FillRect {
    rect: Rect::new(0.0, 0.0, 100.0, 20.0),
    color: Color::rgb(0x33, 0x33, 0x33),
});
// 每个 PushClip 都要有对应的 PopClip —— `clips_balanced()` 可以自检
assert!(scene.clips_balanced());
```

### 交给后端画

```rust
use neo_ui_render::{Color, Op, Rect, RenderBackend, Scene, GpuiBackend};

let mut scene = Scene::new();
scene.push(Op::FillRect {
    rect: Rect::new(0.0, 0.0, 10.0, 10.0),
    color: Color::rgb(0xff, 0x00, 0x00),
});

let mut backend = GpuiBackend::new();
let paint = backend.paint(&scene);
assert_eq!(backend.name(), "gpui");
// `paint` 持有后端相关的绘制数据（这里就是一组 quad）与统计信息
let _ = paint;
```

`RenderBackend::measure_text` 用**显示宽度**而不是字符数 —— 中文字符占两列，
按字符数算会让包含中文的布局整体错位。

### 一个具体的自绘消费者

```rust
use neo_ui_render::{change_gutter, GutterMark};

// 变更条：只认识"有变化 / 新增 / 删除"三态，**不认识 diff 语义**
// （那是调用方的事）—— 所以它不知道什么是 hunk，只知道哪几行要画。
let marks = vec![
    GutterMark::Plain,
    GutterMark::Add,
    GutterMark::Add,
    GutterMark::Del,
];
let scene = change_gutter(&marks, 4.0, 40.0);
assert!(!scene.ops().is_empty());
```

## 设计取舍（写下来是因为它们都反直觉）

**中立类型不用后端的类型。** `Color` / `Point` / `Size` / `Rect` 是本 crate 自己
定义的，不是某个后端的。若直接用后端类型，换后端就要改所有调用方 —— 那这条缝
就白开了。

**`Rect::contains` 是半开区间**（左闭右开）。相邻矩形共享边界时，半开区间让
一个点只属于其中一方，不会两边都算或都不算。

**变更条每格至少 1 个逻辑像素。** 否则长 diff 里的小改动会被取整抹掉 ——
而"有改动却看不见"比"少显示一格"更糟。代价是极端情况下色带总高会超出表面，
所以必须压配平的裁剪（`clips_balanced` 守着这条）。

**场景必须 `Send + 'static` 且可廉价克隆**（便于 diff）。这是 trait 的约束，
不是建议。

## 诚实边界

- **只有一个后端实现**（gpui）。第二个实现（如基于 wgpu 的）**刻意不提前写**：
  在只有一个实现时，第二个实现的所有假设都是错的。
- 绘制指令集目前很小（填充矩形、描边矩形、文字、裁剪栈）。**够用就好**，
  不为想象中的需求预先加指令。
- `GpuiBackend::paint` 返回的数据仍需由调用方在框架的绘制回调里真正提交
  （那里才有绘制上下文）。本 crate 不持有窗口。

## 许可

Apache-2.0，见 [LICENSE](LICENSE)。

选它而不是 MIT 的原因：Apache-2.0 含**明确的专利授权**条款（第 3 节）——
使用者不必另行担心贡献者持有的专利主张。对一个打算被商业项目采用的库来说，
这一点比"文本更短"重要。
