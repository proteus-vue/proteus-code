# neo-text

宿主中立的文本语义：语义色调、调色板、宽度计算、Markdown 解析、语法高亮。

`Tone` 是**语义**角色（"这是强调色"），不是具体颜色。同一个 `Tone` 在终端可能
映射到 ANSI 色、在 GPU 界面映射到某个十六进制值 —— 映射表由各宿主提供，
而"它属于哪个语义角色"由本 crate 定义。

## 三个部分

| 模块 | 作用 |
|---|---|
| `palette` | 语义调色板：`Tone` → RGB 的唯一定义处 |
| `markdown` | Markdown → 带语义色调的行/块，**同一份解析**喂给终端与 GUI |
| `syntax` | 按语言的轻量语法高亮，输出 `Tone` 区间 |
| `width` | 显示宽度（CJK 是双宽，`chars().count()` 会算错） |

## 用法

### 宽度：不要用字符数当列宽

```rust
use neo_text::width;

// 中文字符占两列 —— 用 `chars().count()` 会让表格与对齐全部错位
assert_eq!(width::display_width("abc"), 3);
assert_eq!(width::display_width("中文"), 4);
assert_eq!(width::display_width("中a"), 3);

// 按列宽截断，不会把一个双宽字符劈成半个
let cut = width::truncate_to_width("中文abc", 3);
assert_eq!(width::display_width(&cut), 3);
```

### Markdown：终端渲染

```rust
use neo_text::markdown;

// 按列宽折行，适合等宽终端
let lines = markdown::render("# 标题\n\n这是一段**强调**文字。", 40);
assert!(!lines.is_empty());
```

### Markdown：GUI 按块取

```rust
use neo_text::Tone;
use neo_text::markdown;

// 每行是一串 (文本, 语义色调) 片段，由调用方决定怎么画
let blocks = markdown::blocks("普通 **强调** 普通");
for line in &blocks {
    for (text, tone) in line {
        match tone {
            Tone::Primary => { /* 用突出的正文样式画这段 */ }
            Tone::Muted => { /* 用弱化样式画 */ }
            _ => { /* 用正文样式画 */ }
        }
        let _ = text;
    }
}
```

### 语法高亮

```rust
use neo_text::{syntax, Lang};

let spans = syntax::highlight_line("let x = 1; // 注释", Lang::Rust);
assert!(!spans.is_empty());
```

### 调色板

```rust
use neo_text::palette::NEO;

let (r, g, b) = NEO.rgb(neo_text::Tone::Primary);
let _ = (r, g, b);
```

## 为什么"只定义一次"很重要

它定义**语义**，不定义**外观**。如果每个宿主各自写一份 `Tone` → 颜色的映射表，
同一条消息在终端和窗口里就会长得不一样，而"同一事件流下语义等价"是
多宿主架构的硬要求。所以：

- 色值只有 `palette` 一处（防漂移守卫会检查各宿主不写字面色值）；
- Markdown 解析只有 `markdown` 一处 —— 各宿主各解析一遍必然会分叉出
  "有的宿主支持删除线、有的不支持"这类差别。

## 诚实边界

- **不是排版引擎**：不做双向文本（bidi）、不做字距调整、不做断字。
  折行按显示宽度做，足够等宽终端与简单 GUI 文本块。
- **语法高亮是轻量的**：按行正则/状态匹配，不是完整词法分析。
  复杂嵌套（如 Rust 宏内的字符串）可能着色不准，这是有意的取舍
  （不引入 tree-sitter 级别的依赖）。
- 只有一条外部依赖：`pulldown-cmark`（关闭默认特性，避免拖入 C 后端）。

## 许可

Apache-2.0，见 [LICENSE](https://github.com/proteus-vue/proteus-code/blob/main/crates/neo-text/LICENSE)。

选它而不是 MIT 的原因：Apache-2.0 含**明确的专利授权**条款（第 3 节）——
使用者不必另行担心贡献者持有的专利主张。对一个打算被商业项目采用的库来说，
这一点比"文本更短"重要。
