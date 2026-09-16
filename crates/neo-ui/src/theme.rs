//! NEO 品牌主题：把 `neo-text` 的调色板装进 gpui-kit 的主题系统。
//!
//! # 为什么不是"设几个颜色"这么简单
//!
//! gpui-component 的主题是**两层**的：一层的语义色（`primary` / `accent` / …），
//! 一层派生出的组件 token（`button_primary` / 滚动条 / 拖拽柄…）。
//! 改完第一层必须 `sync_base` 让第二层重算 —— 否则按钮还是旧色，
//! 而"改了主题却只有部分生效"看起来像 bug，实际是漏了一次同步。
//!
//! # 品牌色来自哪里
//!
//! 从 `neo_text::palette::NEO` 读，**不在这里写字面量**。这样：
//! - TUI 的 NEO 主题、egui 宿主、gpui 宿主三处颜色一致（有防漂移守卫守着）；
//! - 改品牌色只改一处（`neo-text` 的 palette），且 TUI 侧会因守卫测试失败而提醒。

use neo_text::{palette, Tone};
use neo_ui_render::Color;

/// 语义色调 → gpui 颜色。
///
/// 走"中立 `Color` → 后端色值"两步，而不是直接从调色板拿 RGB：
/// 这样"Tone 到颜色的映射"只有一条路径，与 `neo-ui-render` 的 `from_tone`
/// 是同一份语义（换后端时不用重写映射）。
pub fn neo_color(tone: Tone) -> neo_ui_kit::gpui::Rgba {
    neo_ui_kit::gpui::rgba(Color::from_tone(&palette::NEO, tone).to_rgba_u32())
}

/// 把 NEO 品牌色装进 gpui-component 的全局主题。
///
/// **必须在 `neo_ui_kit::init(cx)` 之后、开窗之前调用一次。**
/// 之后再调也可以（用于运行时换主题），但要跟着 `refresh_windows` 让界面重绘。
pub fn apply_neo_theme(cx: &mut neo_ui_kit::gpui::App) {
    use neo_ui_kit::component::Theme;

    // 十六进制字符串是 gpui-component 的公开解析入口（支持 #RRGGBB / #RRGGBBAA）。
    // 这里从 `neo-text` 的调色板取数值再格式化成字符串，而不是直接写 "#a78bfa" ——
    // 后者会让品牌色有两个来源，改一处忘一处。
    let hex = |tone: Tone| -> String {
        let (r, g, b) = palette::NEO.rgb(tone);
        format!("#{r:02x}{g:02x}{b:02x}")
    };

    // 语义色：primary 是"最常出现的强调"（品牌紫），accent 是结构性位置
    // （标题、用户消息竖条）——与 `neo-text` 里那条注释同一套取舍。
    set_color(cx, &hex(Tone::Primary), |t| &mut t.primary);
    set_color(cx, &hex(Tone::Accent), |t| &mut t.accent);
    set_color(cx, &hex(Tone::Success), |t| &mut t.success);
    set_color(cx, &hex(Tone::Error), |t| &mut t.danger);
    set_color(cx, &hex(Tone::Warning), |t| &mut t.warning);
    set_color(cx, &hex(Tone::Info), |t| &mut t.info);

    // ⚠️ 必须同步：上面改的是"语义色"那一层，组件 token（按钮底色、
    // 滚动条、拖拽柄…）是从它派生的。不调这一步，界面会一半新色一半旧色。
    Theme::sync_base(cx);
}

/// 设置一个颜色字段（解析失败就保持原值并如实报告，不 panic）。
///
/// 为什么不 `unwrap`：主题色是**外观**，一个格式错误不该让应用起不来 ——
/// 但要**说出来**，否则"颜色没生效"会被当成主题系统的 bug 去查。
fn set_color(
    cx: &mut neo_ui_kit::gpui::App,
    hex: &str,
    pick: impl FnOnce(&mut neo_ui_kit::component::ThemeColor) -> &mut neo_ui_kit::gpui::Hsla,
) {
    let parsed = match neo_ui_kit::component::try_parse_color(hex) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[neo-ui] 主题色 {hex} 解析失败，保持默认值：{e}");
            return;
        }
    };
    let theme = neo_ui_kit::component::Theme::global_mut(cx);
    *pick(theme) = parsed;
}

#[cfg(test)]
mod tests {
    use super::*;
    use neo_ui_kit::component::Theme;
    use neo_ui_kit::gpui::App;

    /// 品牌色必须能从调色板派生出来，且与调色板**逐值一致**。
    ///
    /// 这条是"三宿主同色"的守卫：如果哪天有人在别处硬编码了品牌紫，
    /// 或者调色板改了而这里没跟着，测试会红。
    #[test]
    fn brand_colors_come_from_the_shared_palette() {
        let primary = Color::from_tone(&palette::NEO, Tone::Primary);
        assert_eq!((primary.r, primary.g, primary.b), (0xa7, 0x8b, 0xfa), "品牌紫");
        let accent = Color::from_tone(&palette::NEO, Tone::Accent);
        assert_eq!((accent.r, accent.g, accent.b), (0xe8, 0x79, 0xf9), "强调品红");
    }

    /// `neo_color` 与调色板一致（它是 UI 层取色的唯一入口）。
    #[test]
    fn neo_color_matches_the_palette() {
        let want = Color::from_tone(&palette::NEO, Tone::Accent).to_rgba_u32();
        assert_eq!(neo_color(Tone::Accent), neo_ui_kit::gpui::rgba(want));
    }

    /// 十六进制格式化正确（这是能直接比对的中间产物）。
    #[test]
    fn hex_formatting_is_lowercase_six_digits() {
        let (r, g, b) = palette::NEO.rgb(Tone::Primary);
        let s = format!("#{r:02x}{g:02x}{b:02x}");
        assert_eq!(s, "#a78bfa");
        assert_eq!(s.len(), 7, "必须是 #RRGGBB（7 字符）");
    }

    /// `apply_neo_theme` 在无窗口环境下也要能跑（装主题不依赖窗口）。
    ///
    /// 用一个真实的 gpui `App` 需要平台后端，测试里跑不起来；所以这里
    /// 只验证"解析 + 取全局"这条路径的纯逻辑部分不会 panic。
    #[test]
    fn parsing_the_brand_hex_succeeds() {
        // 与 apply_neo_theme 内部同一条解析路径
        for tone in [Tone::Primary, Tone::Accent, Tone::Success, Tone::Error, Tone::Warning] {
            let (r, g, b) = palette::NEO.rgb(tone);
            let hex = format!("#{r:02x}{g:02x}{b:02x}");
            assert!(
                neo_ui_kit::component::try_parse_color(&hex).is_ok(),
                "调色板导出的 {hex} 应能被主题解析器接受"
            );
        }
    }

    /// 主题类型可达（门面层透出正确）。
    #[test]
    fn theme_type_is_reachable_through_the_facade() {
        let _ = std::any::type_name::<Theme>();
        let _ = std::any::type_name::<App>();
    }
}
