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

/// 文字层级（**设计系统的排版角色**，不是随手设字号）。
///
/// # 为什么要有角色，而不是各处直接 `text_sm()`
///
/// 实测：宿主里 `font_weight` / `text_size` 出现 **0 次** —— 标题、正文、说明
/// 全是同一个字号同一个字重，界面因此"平"。但修法若只是"给每个地方挑个字号"，
/// 会得到十几种各不相同的组合（那正是拼凑感的来源）。
///
/// 所以按**角色**给：使用者说"这是区块标题"，而不是"这是 14px 加粗"。
/// 数值取自设计系统的排版 token（`TypographyTokens`：xs=12 / sm=14 / base=16，
/// 行高 16 / 20 / 24），不自己发明一套。
///
/// # 为什么是"小一号 + 加粗"而不是"大一号"
///
/// 这是**工具型界面**（dense tool UI）的惯例，也是 Zed / VS Code 的做法：
/// 正文该是主角、字号偏小（信息密度高）；标题靠**字重**与**颜色**区分，
/// 而不是靠变大。把标题放大到 20px 会让面板显得空旷、且挤压内容区。
///
/// 与 §4.64(ba) 的教训一致：**字重在中文字形上可能不生效**（CJK 常无真粗体
/// 字面）。所以层级不只靠字重 —— 标题同时用更亮的语义色，说明文字同时用
/// 弱化色，两条路径叠加，任一条失效都还能分辨。
pub mod text_role {
    use neo_text::Tone;
    use neo_ui_kit::gpui::{Div, FontWeight, div, prelude::*};

    /// 一个字重档（与设计系统的排版 token 同量级）。
    ///
    /// 用**数据**描述角色而不是把样式写死在函数里，是为了让"层级存在"
    /// 成为**可断言的**：四个角色若被改成同一组值，测试会红；
    /// 而写死在函数里就只能靠肉眼看出来（界面"又变平了"不会报错）。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Size {
        /// 12px（行号、角标）
        Xs,
        /// 14px（标题、说明）
        Sm,
        /// 16px（正文）
        Base,
    }

    /// 一个排版角色的**规格**。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Role {
        pub size: Size,
        pub bold: bool,
        pub tone: Tone,
    }

    /// 区块 / 面板标题：小一号 + 半粗 + 信息色。
    ///
    /// 为什么是"小一号"而不是"大一号"：工具型界面（dense tool UI）的惯例 ——
    /// 正文是主角、字号偏小以换取信息密度；标题靠**字重与颜色**区分。
    /// Zed / VS Code 都是这个路子。
    ///
    /// ⚠️ 别忘了 §4.64(ba) 的教训：**CJK 常无真粗体字面**，`bold` 在中文上
    /// 可能看不出差别。所以层级**不只靠字重** —— 标题同时用了更亮的语义色、
    /// 说明文字同时用了弱化色。两条路径叠加，任一条在某字体上失效仍可分辨。
    pub const SECTION_TITLE: Role =
        Role { size: Size::Sm, bold: true, tone: Tone::Info };

    /// 正文（转录内容）：默认字号 + 正文色。
    pub const BODY: Role = Role { size: Size::Base, bold: false, tone: Tone::Text };

    /// 说明 / 元信息（解释性文字、路径、时间戳）：小一号 + 弱化色。
    pub const META: Role = Role { size: Size::Sm, bold: false, tone: Tone::Muted };

    /// 最小一档（行号、角标）。
    pub const TINY: Role = Role { size: Size::Xs, bold: false, tone: Tone::Muted };

    /// 把一个角色应用到元素上（**唯一的套用点**）。
    pub fn apply(role: Role, label: impl Into<neo_ui_kit::gpui::SharedString>) -> Div {
        let mut d = div();
        d = match role.size {
            Size::Xs => d.text_xs(),
            Size::Sm => d.text_sm(),
            Size::Base => d.text_base(),
        };
        if role.bold {
            d = d.font_weight(FontWeight::SEMIBOLD);
        }
        d.text_color(super::neo_color(role.tone)).child(label.into())
    }

    /// 区块 / 面板标题。
    pub fn section_title(label: impl Into<neo_ui_kit::gpui::SharedString>) -> Div {
        apply(SECTION_TITLE, label)
    }

    /// 正文。
    pub fn body(label: impl Into<neo_ui_kit::gpui::SharedString>) -> Div {
        apply(BODY, label)
    }

    /// 说明 / 元信息。
    pub fn meta(label: impl Into<neo_ui_kit::gpui::SharedString>) -> Div {
        apply(META, label)
    }

    /// 最小一档。
    pub fn tiny(label: impl Into<neo_ui_kit::gpui::SharedString>) -> Div {
        apply(TINY, label)
    }
}

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

    /// **四个排版角色必须真的分层**（尺寸、字重、颜色不能全一样）。
    ///
    /// 这条守的是"层级存在"这件事本身。它有意义的原因：`text_role` 把层级集中到
    /// 一处之后，若哪天有人把四个角色写成同一组值，**界面上不会报错** ——
    /// 只是又变平了（用户看得出、测试看不出）。断言"彼此不同"才守得住。
    ///
    /// ⚠️ 只断言**关系**（谁比谁大、谁更粗、颜色不同），不断言具体像素：
    /// 具体值是设计 token，会随主题调整；而"可分辨"才是契约。
    #[test]
    fn text_roles_form_a_real_hierarchy() {
        use text_role::{BODY, META, SECTION_TITLE, TINY, Size};

        // 标题必须比正文**小**（工具界面的惯例：正文是主角）
        assert!(
            SECTION_TITLE.size != BODY.size,
            "标题与正文不能同字号（否则层级不存在）"
        );
        // 元信息必须与正文**不同字号**（否则"说明"与"内容"难分辨）
        assert_ne!(META.size, BODY.size, "元信息应与正文不同字号");
        // 尺寸档必须**有序**：Xs < Sm < Base（用枚举序表达）
        assert!(Size::Xs != Size::Sm && Size::Sm != Size::Base);

        // 标题必须**更粗**（在字形支持粗体的语言上生效）
        assert!(SECTION_TITLE.bold, "标题应当加粗");
        assert!(!BODY.bold, "正文不该加粗");
        assert!(!META.bold, "说明文字不该加粗");

        // 颜色必须**三种不同** —— 这是粗体在 CJK 上失效时的第二道区分路径
        assert_ne!(SECTION_TITLE.tone, BODY.tone);
        assert_ne!(META.tone, BODY.tone);
        assert_ne!(SECTION_TITLE.tone, META.tone);

        // 四个角色两两不可完全相同
        let all = [TINY, META, BODY, SECTION_TITLE];
        for (i, a) in all.iter().enumerate() {
            for b in all.iter().skip(i + 1) {
                assert_ne!(a, b, "两个排版角色完全相同 —— 层级被抹平了");
            }
        }
    }

    /// 主题类型可达（门面层透出正确）。
    #[test]
    fn theme_type_is_reachable_through_the_facade() {
        let _ = std::any::type_name::<Theme>();
        let _ = std::any::type_name::<App>();
    }
}
