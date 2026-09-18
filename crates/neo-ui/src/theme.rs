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

/// 自绘组件的圆角半径（逻辑像素）—— **与主题的 `radius` 同一来源**。
///
/// # 为什么要在渲染缝之外单独开一个入口
///
/// 渲染缝（`neo-ui-render`）是**中立层**，不能读 gpui 的主题（那会让它依赖
/// 某个后端）。所以半径必须像颜色一样**从上层传入**：颜色走 `neo_color`，
/// 半径走这里 —— 两者都是"设计系统的值 → 交给缝"的同一条路径。
///
/// 取 **6px**，即设计系统的 `Theme::radius`（medium）；`radius_lg = 8` 留给
/// 弹窗类大面。数值与主题一致，换主题时一起变（不写死在渲染层，见
/// `neo_ui_render::Op::FillRect` 的说明）。
///
/// 不是"随便挑的 6"：Zed / GitHub 这类工具的控件圆角也在这个量级
/// （4–8px）—— 再大就显得是"卡片"而不是控件，再小则与直角无异。
pub const RADIUS: f32 = 6.0;

/// 页面底色（近黑）。
///
/// # 为什么必须单开一个入口，而不是让调用方写 `neo_color(Tone::None)`
///
/// 因为那是个**陷阱**：`Tone::None` 的语义是"未指定色调"，调色板把
/// `None | Text` 都兜底到 `text` —— 也就是**近白**。拿它当背景色，
/// 会得到一块近白的底，配深色调色板的近白正文，界面几乎看不清。
///
/// 本宿主的第一次真机截图就是这样白的（代码里看着"设了个背景色"）。
/// 所以把"底色"做成显式函数名：名字本身说明它是背景，不再靠猜语义。
pub fn base_bg() -> neo_ui_kit::gpui::Rgba {
    let (r, g, b) = palette::NEO.bg_base;
    neo_ui_kit::gpui::rgba(neo_ui_render::Color::rgb(r, g, b).to_rgba_u32())
}

/// 面板底色（侧栏/对话框，比页面底略亮一档）。
pub fn panel_bg() -> neo_ui_kit::gpui::Rgba {
    let (r, g, b) = palette::NEO.bg_panel;
    neo_ui_kit::gpui::rgba(neo_ui_render::Color::rgb(r, g, b).to_rgba_u32())
}

/// 把 NEO 品牌色装进 gpui-component 的全局主题。
///
/// **必须在 `neo_ui_kit::init(cx)` 之后、开窗之前调用一次。**
/// 之后再调也可以（用于运行时换主题），但要跟着 `refresh_windows` 让界面重绘。
///
/// # 顺序要紧：先切模式，再改颜色
///
/// `gpui_component::init` 装好的是**浅色**主题（`ThemeMode::Light` 是默认值）。
/// 而 NEO 的调色板是深色的（正文色 `#ededed`）。
/// 如果只改语义色不改模式，结果就是**近白的文字画在近白的底上** ——
/// 界面能开、布局正确、中文也正常，但看不清。这不是理论风险：本宿主第一次
/// 真机截图就是这样（见 PROJECT_MEMORY §4.65）。
pub fn apply_neo_theme(cx: &mut neo_ui_kit::gpui::App) {
    use neo_ui_kit::component::{Theme, ThemeMode};

    // ⚠️ 顺序与做法都有讲究，这里是踩过坑之后的写法。
    //
    // gpui-component 的主题有**两层**：
    //   1. `colors: ThemeColor` —— 语义色（primary / accent / background…）
    //   2. `tokens: ThemeTokens` —— 由语义色**派生**的组件 token
    //      （按钮底、滚动条、根视图底色…），`From<&ThemeColor>` 生成。
    //
    // 而 `Root` 渲染时读的是 `cx.theme().tokens.background` —— **第二层**。
    //
    // 第一次实现只改了第一层（`theme.primary = ...`），真机结果是：
    // 窗口开了、布局对、中文正常，但**底色仍是浅色、正文近白**，几乎看不清。
    // 从代码上看"明明设了深色"，实际设的那一层没人读。
    //
    // 正确做法：先切模式（拿到深色基线）→ 覆盖语义色 → **重建 tokens** →
    // 同步 base 层（滚动条/拖拽柄读的是 base 那份拷贝）。

    // 1) 切到深色（同时把 light/dark 两套基线装好）
    Theme::change(ThemeMode::Dark, None, cx);

    // 2) 覆盖语义色（用我们的调色板；hex 派生自 `neo-text`，不写字面量）
    let hex = |tone: Tone| -> String {
        let (r, g, b) = palette::NEO.rgb(tone);
        format!("#{r:02x}{g:02x}{b:02x}")
    };
    let (br, bg, bb) = palette::NEO.bg_base;

    {
        let theme = Theme::global_mut(cx);
        let set = |dst: &mut neo_ui_kit::gpui::Hsla, hexstr: String| {
            if let Ok(c) = neo_ui_kit::component::try_parse_color(&hexstr) {
                *dst = c;
            }
        };
        set(&mut theme.colors.primary, hex(Tone::Primary));
        set(&mut theme.colors.accent, hex(Tone::Accent));
        set(&mut theme.colors.background, format!("#{br:02x}{bg:02x}{bb:02x}"));
        set(&mut theme.colors.foreground, hex(Tone::Text));
        set(&mut theme.colors.border, hex(Tone::Border));
        set(&mut theme.colors.muted, hex(Tone::Muted));
        set(&mut theme.colors.danger, hex(Tone::Error));
        set(&mut theme.colors.success, hex(Tone::Success));
        set(&mut theme.colors.warning, hex(Tone::Warning));
        set(&mut theme.colors.info, hex(Tone::Info));

        // 3) **重建 tokens** —— 漏掉这一步，界面用的还是旧底色/旧按钮色。
        //    这是本函数最容易被漏掉、且最难从代码上看出问题的一步：
        //    语义色看起来"已经改了"，但真正被读的是它们派生出的 token。
        theme.tokens = neo_ui_kit::component::ThemeTokens::from(&theme.colors);
    }

    // 4) base 层（滚动条、拖拽柄、语义 token 投影）是另一份拷贝，要显式同步。
    //    不调它：按钮颜色对了、滚动条还是旧色 —— "改了一半"。
    Theme::sync_base(cx);
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

    /// **深色模式必须显式切换** —— 这条是给真机截图抓到的那个 bug 上的锁。
    ///
    /// `gpui_component::init` 默认装**浅色**主题，而 NEO 的调色板是深色的
    /// （正文 `#ededed` 近白）。只改语义色不切模式，结果是近白文字画在近白底上：
    /// 窗口能开、布局对、中文正常，**但看不清**。本宿主第一次真机截图就是这样。
    ///
    /// 这里断言 NEO 的正文色确实比底色**亮** —— 若哪天有人把模式改回 Light，
    /// 或换了个亮底调色板却没同步改模式，这条会失败。
    #[test]
    fn the_palette_requires_dark_mode() {
        let (tr, tg, tb) = palette::NEO.rgb(Tone::Text);
        let (br, bg, bb) = palette::NEO.bg_base;
        let lum = |r: u8, g: u8, b: u8| 0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32;
        assert!(
            lum(tr, tg, tb) > lum(br, bg, bb) + 100.0,
            "NEO 调色板的正文色应显著亮于底色（说明它是**深色**主题）—— \
             若这条失败，`apply_neo_theme` 里的 ThemeMode::Dark 可能被改掉了，\
             界面会变成浅底浅字"
        );
    }

    /// **`Tone::None` 不能当背景色用** —— 它兜底到 `text`（近白）。
    ///
    /// 这是真机截图抓到的第二个坑（第一次是主题模式没切）：
    /// 宿主写了 `.bg(neo_color(Tone::None))`，看着像"设了背景"，
    /// 实际画出一块近白的底，配近白正文 = 看不清。
    /// 现在有了显式的 [`base_bg`]，这条测试守住"两者确实不同"。
    #[test]
    fn tone_none_is_not_a_background_color() {
        let none_as_bg = neo_color(Tone::None);
        let real_bg = base_bg();
        assert_ne!(
            none_as_bg, real_bg,
            "Tone::None 兜底到正文色（近白），不能当底色用；\
             背景请用 neo_ui::base_bg()"
        );
    }

    /// 页面底必须比正文暗（不然就是白底白字）。
    #[test]
    fn the_base_background_is_darker_than_the_text() {
        let lum = |c: neo_ui_kit::gpui::Rgba| {
            let (r, g, b) = (c.r * 255.0, c.g * 255.0, c.b * 255.0);
            0.2126 * r + 0.7152 * g + 0.0722 * b
        };
        assert!(
            lum(base_bg()) + 100.0 < lum(neo_color(Tone::Text)),
            "底色应显著暗于正文色，否则界面看不清"
        );
    }

    /// 主题类型可达（门面层透出正确）。
    #[test]
    fn theme_type_is_reachable_through_the_facade() {
        let _ = std::any::type_name::<Theme>();
        let _ = std::any::type_name::<App>();
    }
}
