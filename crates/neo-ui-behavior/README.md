# neo-ui-behavior

与渲染后端无关的 UI 行为逻辑。去掉颜色和尺寸之后仍然成立的那部分界面规则。

## 这个 crate 解决什么

三类界面问题**和用什么渲染框架无关**，但几乎每个含自绘界面的程序都会各自踩一遍：

| 问题 | 本 crate 的答案 |
|---|---|
| 多个地方同时想抢焦点，谁赢取决于代码执行顺序 | `FocusIntent<T>`：单一待处理槽位，后设置者生效 |
| 快捷键被框架自己的按键处理抢先消费 | `KeyArbiter`：按层级裁决按键归属 |
| 轮次耗时怎么算，才不会污染可回放的状态 | `TurnClock`：时间由外部注入，逻辑不读挂钟 |

这些逻辑不依赖任何 GUI 框架，因此可以在无窗口环境里测试 —— 每一条都有覆盖边界情况的单元测试（时钟倒退、重复 `start`、模态独占键盘等）。

## 用法

### FocusIntent —— 焦点意图

用一个泛型参数表示焦点目标，由调用方定义：

```rust
use neo_ui_behavior::FocusIntent;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Target { Composer, Palette }

let mut intent: FocusIntent<Target> = FocusIntent::new();
intent.request(Target::Palette);          // 动作显式指定焦点
intent.request_if_none(Target::Composer); // 兜底：不覆盖上面那个显式请求

// 渲染时**消费**（take 而不是 peek）：每帧至多一次。
// 若改成每帧都读，用户一移开焦点就会被拽回来。
assert_eq!(intent.take(), Some(Target::Palette));
assert_eq!(intent.take(), None);
```

### KeyArbiter —— 按键归属

先声明哪些键由应用接管，再按当前层问某个键归谁：

```rust
use neo_ui_behavior::{KeyArbiter, Layer, Verdict};

let arbiter = KeyArbiter::new()
    .claim("shift+tab")   // 全局键：应用要它
    .claim("escape");     // Esc 关面板

// 普通层：应用声明过的键归应用，其余交给框架（正常打字）
assert_eq!(arbiter.verdict(Layer::Normal, "shift+tab", false), Verdict::App);
assert_eq!(arbiter.verdict(Layer::Normal, "a", false), Verdict::Framework);

// 输入焦点下，全局键**仍然**归应用
assert_eq!(arbiter.verdict(Layer::Focused, "shift+tab", true), Verdict::App);

// 模态层独占键盘：非文本键不让全局快捷键穿透
assert_eq!(arbiter.verdict(Layer::Modal, "f5", false), Verdict::App);
// 但模态里若有输入框，文本输入仍归框架
assert_eq!(arbiter.verdict(Layer::Modal, "a", true), Verdict::Framework);
```

### TurnClock —— 轮次计时

时间由调用方提供（逻辑不读挂钟，因此结果可复现）：

```rust
use std::time::Duration;
use neo_ui_behavior::TurnClock;

let mut clock = TurnClock::new();
assert_eq!(clock.elapsed_since_start(Duration::from_secs(1)), None); // 还没开始

clock.start(Duration::from_secs(10));
clock.start(Duration::from_secs(12)); // 重复 start 取**最早**那次
assert_eq!(
    clock.elapsed_since_start(Duration::from_millis(10_250)),
    Some(Duration::from_millis(250)),
);

clock.finish();
assert!(!clock.is_running());
```

未开始时返回 `None` 而不是 `Duration::ZERO` —— "还没开始"与"耗时为零"是两件事，
混为一谈会让界面显示出并不存在的 `0.0s`。`format_duration` 同样不会输出 `0.0s`。

## 诚实边界

- **它不是组件库**：不含视觉元素，不做布局与渲染。
- `KeyArbiter` 只**裁定**归属，不负责真正拦住框架。实际拦截要在各框架的
  事件钩子里做，那部分是框架专有代码，在别处。
- **不预留扩展点**：目前只有三个模块，加新东西时再设计。

## 许可

Apache-2.0，见 [LICENSE](LICENSE)。

选它而不是 MIT 的原因：Apache-2.0 含**明确的专利授权**条款（第 3 节）——
使用者不必另行担心贡献者持有的专利主张。对一个打算被商业项目采用的库来说，
这一点比"文本更短"重要。
