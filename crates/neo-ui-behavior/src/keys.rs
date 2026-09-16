//! 按键路由：**模态优先**，且由应用自己裁决，而不是让框架先吃掉。
//!
//! # 它修掉的那类 bug
//!
//! egui 版实测遇到两次同类问题，根因相同、表现不同：
//!
//! - `Shift+Tab` 被框架当作**焦点导航键**：模式切换确实发生了，但焦点同时
//!   从输入框跳到了右侧面板的 resize 手柄，**接着敲的字全部丢失**。
//! - `Esc` 被框架当作**结束文本编辑**：搜索框被清空了，但面板**还开着** ——
//!   用户按 Esc 以为关掉了，实际没有。
//!
//! 两次的修法都一样：在框架读按键**之前**把事件摘掉（egui 里是
//! `raw_input_hook`）。但这属于"知道内情才写得对"的代码，换个框架要重新踩一遍。
//!
//! 所以这里把规则独立出来：**给定当前界面所处的层，这个键归谁**。
//! 框架相关的部分（怎么摘事件）留在宿主，规则与测试留在本层。
//!
//! # 为什么要分层（而不是"谁先注册谁处理"）
//!
//! 模态界面（对话框、命令面板）存在的意义就是**暂时接管键盘**。
//! 若按键按注册顺序处理，那么先注册的全局快捷键会穿透模态 ——
//! 表现为"对话框开着，快捷键照样触发"，这几乎总是错的。

/// 界面所处的层。
///
/// ⚠️ **变体的声明顺序就是优先级顺序**（`derive(Ord)` 按声明先后取值），
/// 所以 `Normal` 排在最前、`Modal` 排在最后 —— 这样 `Layer::Modal` 是最大值，
/// `layers.iter().max()` 给出"当前最该拿到按键的那一层"。
/// 反过来声明会让 `.max()` 选出 `Normal`，而那是**最底层** —— 一个方向写错
/// 就变成"模态被普通层压制"，没有任何编译错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    /// 普通界面（优先级最低）。
    Normal,
    /// 输入焦点所在（输入框/编辑器）。普通按键进文本，全局键仍可能生效。
    Focused,
    /// 模态：对话框、命令面板、审批确认。**独占键盘**（优先级最高）。
    Modal,
}

/// 一个键的归属裁定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// 归应用处理（框架不该先消费它）。
    App,
    /// 交给框架（文本输入、焦点导航等）。
    Framework,
    /// 谁都不处理（丢掉）。
    Ignore,
}

/// 按键路由表：按当前层裁决按键归属。
///
/// 它是**纯逻辑**（不认识任何框架的按键类型），因此可以完整单测 ——
/// 而"按键被框架抢先吃掉"这类问题恰恰最需要测试：它只在真机上暴露，
/// 且现象（"打字没了""按了没反应"）很难反向联想到按键归属。
#[derive(Debug, Clone, Default)]
pub struct KeyArbiter {
    /// 应用自己接管的键（用调用方的键标识，如 `"shift+tab"`）。
    app_keys: Vec<String>,
}

impl KeyArbiter {
    pub fn new() -> Self {
        Self { app_keys: Vec::new() }
    }

    /// 声明一个由应用接管的键（如 `"shift+tab"` 切换执行模式）。
    pub fn claim(mut self, key: &str) -> Self {
        self.app_keys.push(key.to_ascii_lowercase());
        self
    }

    /// 裁决：在 `layer` 层下，`key` 归谁。
    ///
    /// 规则（按优先级）：
    /// 1. **模态层**：应用声明的键归应用；其余按键如果在"文本类"白名单里
    ///    就给框架（模态里也可能有输入框），否则应用优先。
    /// 2. **有输入焦点**：应用声明的键仍归应用（`Shift+Tab` 之类全局键
    ///    在输入框聚焦时也该生效 —— ZCode 就是这么定的）；
    ///    其余按键给框架（正常的文字输入）。
    /// 3. **普通层**：应用声明的键归应用，其余给框架。
    pub fn verdict(&self, layer: Layer, key: &str, is_text_input: bool) -> Verdict {
        let k = key.to_ascii_lowercase();
        let app_claims = self.app_keys.iter().any(|c| *c == k);

        match layer {
            Layer::Modal => {
                if app_claims {
                    // 模态里应用声明的键优先（Esc 关面板就属于这类）
                    Verdict::App
                } else if is_text_input {
                    Verdict::Framework
                } else {
                    // 非文本键：模态层吞掉，不让全局快捷键穿透
                    Verdict::App
                }
            }
            Layer::Focused | Layer::Normal => {
                if app_claims {
                    Verdict::App
                } else {
                    Verdict::Framework
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arbiter() -> KeyArbiter {
        // 应用接管的键：与 egui 版实际实现的一致
        KeyArbiter::new()
            .claim("shift+tab") // 循环切换执行模式
            .claim("cmd+k") // 命令面板
            .claim("esc") // 关闭覆盖层
    }

    #[test]
    fn claimed_keys_always_go_to_the_app() {
        let a = arbiter();
        for layer in [Layer::Modal, Layer::Focused, Layer::Normal] {
            assert_eq!(a.verdict(layer, "shift+tab", false), Verdict::App, "{layer:?}");
            assert_eq!(a.verdict(layer, "cmd+k", false), Verdict::App, "{layer:?}");
        }
    }

    #[test]
    fn key_matching_is_case_insensitive() {
        let a = arbiter();
        assert_eq!(a.verdict(Layer::Normal, "Shift+Tab", false), Verdict::App);
        assert_eq!(a.verdict(Layer::Normal, "CMD+K", false), Verdict::App);
    }

    /// **模态独占键盘**：普通键不让全局快捷键穿透。
    ///
    /// 这条是"对话框开着、快捷键照样触发"那类 bug 的防线。
    #[test]
    fn modal_swallows_unclaimed_keys() {
        let a = arbiter();
        assert_eq!(
            a.verdict(Layer::Modal, "f5", false),
            Verdict::App,
            "模态里的普通键应被吞掉，不让它穿透到全局快捷键"
        );
    }

    /// 但模态里的**文本键**要给框架 —— 模态里也可能有输入框。
    #[test]
    fn modal_still_lets_text_through_to_an_input() {
        let a = arbiter();
        assert_eq!(
            a.verdict(Layer::Modal, "a", true),
            Verdict::Framework,
            "模态里聚焦输入框时，普通字符该进文本"
        );
    }

    /// **输入框聚焦时，全局键仍归应用**（ZCode 的 `Shift+Tab` 就是在
    /// 输入框聚焦时循环切换模式的）。
    #[test]
    fn global_keys_still_work_while_typing() {
        let a = arbiter();
        assert_eq!(
            a.verdict(Layer::Focused, "shift+tab", true),
            Verdict::App,
            "在输入框里按 Shift+Tab 也该切模式"
        );
    }

    #[test]
    fn unclaimed_keys_go_to_the_framework_for_text_input() {
        let a = arbiter();
        assert_eq!(a.verdict(Layer::Focused, "a", true), Verdict::Framework);
        assert_eq!(a.verdict(Layer::Normal, "a", false), Verdict::Framework);
    }

    /// 层是有序的：Modal > Focused > Normal。调用方据此挑"当前层"。
    #[test]
    fn layers_are_ordered_so_the_topmost_wins() {
        assert!(Layer::Modal > Layer::Focused);
        assert!(Layer::Focused > Layer::Normal);
        // 实际用法：取当前活跃层里最高的那个
        let active = [Layer::Normal, Layer::Modal, Layer::Focused];
        assert_eq!(active.iter().max(), Some(&Layer::Modal));
    }
}
