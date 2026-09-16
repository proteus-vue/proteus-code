//! egui 视觉风格：把语义调色板装进 `egui::Style`。
//!
//! # 为什么需要这一层
//!
//! egui 自带一套浅色默认样式（灰白底、圆角、蓝色高亮）。不覆盖的话，
//! 我们的深色语义调色板只作用在**我们亲手画的文字**上，而按钮、输入框、
//! 滚动条仍是 egui 的浅色皮肤 —— 一个窗口里两种视觉语言，看起来就是
//! "没做完"。这类问题 TUI 侧也遇到过（PROJECT_MEMORY §4.47：底色层是"面"、
//! 边框只是"线"）。
//!
//! 所以这里把调色板铺到 egui 的各个视觉槽位上：底色、面板、输入框、选中态、
//! 描边。**只做映射，不发明颜色** —— 颜色值全部来自 `neo-text` 的调色板，
//! 与 TUI 同源。

use crate::GuiPalette;
use crate::Tone;

/// 把 NEO 调色板装进 egui 的全局 Style。
pub fn install(ctx: &egui::Context) {
    let p = GuiPalette::neo();
    // egui 0.36 把样式拆成 dark/light 两份（`style_of` / `set_style_of`），
    // 所以按主题分别装 —— 只改一份会在另一份上残留默认皮肤。
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();

    style.visuals = visuals(&p);
    // 字号略大：原生窗口的 DPI 与视距都与终端不同，终端字号在窗口里偏小
    style.text_styles = [
        (egui::TextStyle::Heading, egui::FontId::proportional(20.0)),
        (egui::TextStyle::Body, egui::FontId::proportional(14.0)),
        (egui::TextStyle::Monospace, egui::FontId::monospace(13.0)),
        (egui::TextStyle::Button, egui::FontId::proportional(14.0)),
        (egui::TextStyle::Small, egui::FontId::proportional(11.5)),
    ]
    .into();

    ctx.set_style_of(egui::Theme::Dark, style.clone());
    // 亮色也装同一套：我们只做深色语义风格，若系统是亮色主题而这里不覆盖，
    // 窗口会一半深一半浅。宁可两处一致，也不要"跟随系统"的半成品。
    ctx.set_style_of(egui::Theme::Light, style);
    // 固定用深色主题（我们的调色板是深色的）
    ctx.set_theme(egui::Theme::Dark);
}

/// 深色语义风格的 `Visuals`。
pub fn visuals(p: &GuiPalette) -> egui::Visuals {
    let mut v = egui::Visuals::dark();

    let base = p.base_bg();
    let panel = p.panel_bg();
    let element = p.element_bg();
    let selected = p.selected_bg();
    let text = p.color(Tone::Text);
    let muted = p.color(Tone::Muted);
    let border = p.color(Tone::Border);
    let accent = p.color(Tone::Primary);

    // 底色阶梯：window(页面) → panel(面板) → extreme/faint(元素)
    v.window_fill = base;
    v.panel_fill = base;
    v.extreme_bg_color = element;
    v.faint_bg_color = panel;

    // 文本层次
    v.override_text_color = Some(text);

    // 描边：边框是"线"，用 Muted/Border 一档，不要抢前景
    v.window_stroke = egui::Stroke::new(1.0, border);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.bg_stroke = egui::Stroke::new(1.0, border);
        w.fg_stroke = egui::Stroke::new(1.0, muted);
    }

    // 交互态：从"静"到"活"逐级提亮，选中/悬停用品牌色系
    v.widgets.inactive.weak_bg_fill = panel;
    v.widgets.inactive.bg_fill = panel;
    v.widgets.hovered.weak_bg_fill = selected;
    v.widgets.hovered.bg_fill = selected;
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, text);
    v.widgets.active.weak_bg_fill = selected;
    v.widgets.active.bg_fill = selected;
    v.widgets.active.fg_stroke = egui::Stroke::new(1.0, accent);

    // 选中/强调：品牌紫
    v.selection.bg_fill = selected;
    v.selection.stroke = egui::Stroke::new(1.0, accent);
    v.hyperlink_color = p.color(Tone::Info);
    // 高亮（如搜索结果）用半透明强调色，避免盖住文字
    v.selection.stroke = egui::Stroke::new(1.0, accent);

    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_mode_visuals_do_not_keep_egui_default_light_surfaces() {
        // 关键：覆盖后不得残留 egui 的浅色默认面 —— 那会让窗口里出现
        // "一半深色一半浅色"。这里断言底色确实来自我们的调色板。
        let p = GuiPalette::neo();
        let v = visuals(&p);
        assert_eq!(v.window_fill, p.base_bg());
        assert_eq!(v.panel_fill, p.base_bg());
        assert_eq!(v.extreme_bg_color, p.element_bg());
        assert_eq!(v.faint_bg_color, p.panel_bg());
        // 深色模式下 egui 默认面板是深灰 #1b1b1b；必须已被我们替换
        assert_ne!(v.panel_fill, egui::Color32::from_rgb(0x1b, 0x1b, 0x1b));
    }

    #[test]
    fn text_color_is_the_palette_text_not_egui_default() {
        let p = GuiPalette::neo();
        let v = visuals(&p);
        assert_eq!(v.override_text_color, Some(p.color(Tone::Text)));
    }

    #[test]
    fn interactive_states_are_visibly_distinct() {
        // 悬停/激活与静止态必须有可见差别，否则"点上去没反应"
        let p = GuiPalette::neo();
        let v = visuals(&p);
        assert_ne!(
            v.widgets.inactive.bg_fill, v.widgets.hovered.bg_fill,
            "悬停态必须与静止态可区分"
        );
        assert_ne!(
            v.widgets.hovered.fg_stroke.color, v.widgets.inactive.fg_stroke.color,
            "悬停时前景也要提亮"
        );
    }

    #[test]
    fn install_sets_the_style_without_panicking() {
        // 在无窗口环境下也应当可用（egui::Context 不依赖窗口）
        let ctx = egui::Context::default();
        install(&ctx);
        // 深色与亮色两份都要装（只装一份会残留默认皮肤）
        for t in [egui::Theme::Dark, egui::Theme::Light] {
            let style = ctx.style_of(t);
            assert_eq!(
                style.visuals.panel_fill,
                GuiPalette::neo().base_bg(),
                "{t:?} 主题未装上我们的调色板"
            );
        }
    }
}
