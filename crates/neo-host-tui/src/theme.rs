//! 主题 —— 对齐 opencode 的多主题体系并可切换
//!
//! # 为什么值得做成"可切换"而不是写死几个颜色
//!
//! 写死颜色等于替用户决定了对比度是否够了。终端的背景色、字体、
//! 用户的色觉都不一样；给出多套经过设计的配色让人自己挑，
//! 比我们猜一套更诚实。
//!
//! 色值取自已核对的 opencode 主题文件（`theme/assets/*.json` 的
//! dark 分支），每个主题都补齐我们特有的两个颜色：
//!   - `star_dim` / `star_bright`：星场两档亮度（opencode 没有星场）
//!   - `border_active`：边框高亮（opencode 的 borderActive 映射）
//!
//! # 不引依赖
//!
//! 解 JSON 用 `serde_json`，但主题是编译期常量表 —— 启动时不需要
//! 读文件、解析、容错。用户选择只需记住一个名字。

use serde::{Deserialize, Serialize};

/// 一套主题的调色板（RGB）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
    pub primary: (u8, u8, u8),
    pub accent: (u8, u8, u8),
    pub success: (u8, u8, u8),
    pub error: (u8, u8, u8),
    pub warning: (u8, u8, u8),
    pub info: (u8, u8, u8),
    pub text: (u8, u8, u8),
    pub muted: (u8, u8, u8),
    pub border: (u8, u8, u8),
    pub border_active: (u8, u8, u8),
    /// 星场暗星（比 border 更暗，避免背景抢注意力）
    pub star_dim: (u8, u8, u8),
    /// 星场亮星
    pub star_bright: (u8, u8, u8),
    /// 面板底色（侧栏、设置页底）。比终端底色略亮，才能看出"这是一块面"。
    pub bg_panel: (u8, u8, u8),
    /// 选中行底色（低饱和强调色，不抢前景可读性）。
    pub bg_selected: (u8, u8, u8),
    /// **元素**底色：比面板再亮一档，用于"底部选项条"这类二级容器。
    ///
    /// opencode 的层级是一条固定亮度阶梯（step1 页面 → step2 面板 → step3 元素）。
    /// 缺了 element 这一档，对话框就没法把"内容区"与"操作区"分开 ——
    /// 那是它看起来有层次的关键之一。见 docs/opencode-parity.md §1.1。
    pub bg_element: (u8, u8, u8),
    /// 菜单项底色（未选中的药丸）。比 element 略暗，让选中项能"跳出来"。
    pub bg_menu: (u8, u8, u8),
    /// 模态遮罩色：压暗背景用。亮色主题给浅灰（黑遮罩在亮底上是刺眼的洞）。
    pub backdrop: (u8, u8, u8),
    /// 整屏底色：**亮色主题**用它铺满全屏（暗色主题保持终端默认底，
    /// 此值不使用）。
    pub base: (u8, u8, u8),
    /// 亮色主题：整屏铺亮底、文字用深色调（与暗色主题是两种观感，不是换色）。
    pub light: bool,
    /// 是否渲染背景纹理（星场等）。亮色/CRT 主题关闭。
    pub stars: bool,
    /// 方角边框（┌┐└┘）：CRT 终端风的招牌特征；默认圆角（╭╮╰╯）。
    pub square: bool,
}

/// 主题名就是用户可见的标识，也是持久化的键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeName {
    /// NEO 自有配色（默认）：紫为主色，紫→品红渐变
    Neo,
    OpenCode,
    Nord,
    Gruvbox,
    RosePine,
    Tokyonight,
    Catppuccin,
    /// 亮色主题：整屏亮底 + 深色文字（风格而非配色）
    Light,
    /// CRT 绿磷终端风：方角边框、无纹理、通体磷绿
    Terminal,
}

impl Default for ThemeName {
    fn default() -> Self {
        Self::Neo
    }
}

impl Theme {
    /// 角部字形：圆角（默认）或方角（CRT 风）。(TL, TR, BL, BR)。
    pub fn corners(self) -> (&'static str, &'static str, &'static str, &'static str) {
        if self.square {
            ("┌", "┐", "└", "┘")
        } else {
            ("╭", "╮", "╰", "╯")
        }
    }
}

impl ThemeName {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Neo => "neo",
            Self::OpenCode => "opencode",
            Self::Nord => "nord",
            Self::Gruvbox => "gruvbox",
            Self::RosePine => "rosepine",
            Self::Tokyonight => "tokyonight",
            Self::Catppuccin => "catppuccin",
            Self::Light => "light",
            Self::Terminal => "terminal",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        Some(match s.as_str() {
            // `default` 仍解析到 NEO 自有配色（它是默认主题）
            "neo" | "default" => Self::Neo,
            "opencode" => Self::OpenCode,
            "nord" => Self::Nord,
            "gruvbox" => Self::Gruvbox,
            "rosepine" | "rose-pine" => Self::RosePine,
            "tokyonight" | "tokyo-night" => Self::Tokyonight,
            "catppuccin" | "catppuccin-mocha" => Self::Catppuccin,
            "light" | "亮色" => Self::Light,
            "terminal" | "crt" | "green" => Self::Terminal,
            _ => return None,
        })
    }

    /// 按名字顺序列出（切换键遍历用；顺序稳定，用户能习惯）。
    pub fn all() -> [ThemeName; 9] {
        [
            Self::Neo,
            Self::OpenCode,
            Self::Nord,
            Self::Gruvbox,
            Self::RosePine,
            Self::Tokyonight,
            Self::Catppuccin,
            Self::Light,
            Self::Terminal,
        ]
    }

    /// 下一个主题（循环）。
    pub fn next(self) -> Self {
        let all = Self::all();
        let i = all.iter().position(|t| *t == self).unwrap_or(0);
        all[(i + 1) % all.len()]
    }
}

/// 取主题的调色板。
pub fn get(name: ThemeName) -> Theme {
    match name {
        // 默认：opencode darkStep* 系列（主色暖橙 #fab283、强调紫 #9d7cd8）
        // NEO 自有配色：紫是主色。
        //
        // 为什么把紫放在 primary 而不是 accent：`primary` 承担的是**最常出现
        // 的强调**（代码里的函数名、列表圆点、弹窗选中标记、logo 渐变起点），
        // 这些位置的颜色就是"这个产品的颜色"。`accent` 只用在标题、用户消息
        // 竖条这类结构性位置，用同色系的品红把渐变拉开层次。
        ThemeName::Neo => Theme {
            name: "neo",
            primary: (0xa7, 0x8b, 0xfa),      // violet 400，主品牌紫
            accent: (0xe8, 0x79, 0xf9),       // fuchsia 400，渐变终点/标题
            success: (0x6e, 0xe7, 0xb7),
            error: (0xf8, 0x71, 0x71),
            warning: (0xfb, 0xbf, 0x24),
            info: (0x7d, 0xd3, 0xfc),
            text: (0xed, 0xed, 0xed),
            muted: (0x8a, 0x8a, 0x94),       // 略带紫调的灰，与主色同族
            border: (0x45, 0x45, 0x52),
            border_active: (0x5c, 0x5c, 0x6e),
            star_dim: (0x2b, 0x2b, 0x33),
            star_bright: (0x3f, 0x3f, 0x4d),
            bg_panel: (0x20, 0x1e, 0x2a),
            bg_selected: (0x33, 0x2f, 0x45),
            bg_element: (0x2a, 0x27, 0x36),
            bg_menu: (0x26, 0x23, 0x30),
            backdrop: (0x0a, 0x0a, 0x0c),
            base: (0x00, 0x00, 0x00),
            light: false,
            stars: true,
            square: false,
        },
        // 参考主题：opencode 官方暗色（暖橙主色 #fab283 + 紫强调 #9d7cd8）。
        // 保留它是为了可对照 —— 但 NEO 的默认是自己的紫。
        ThemeName::OpenCode => Theme {
            name: "opencode",
            primary: (0xfa, 0xb2, 0x83),
            accent: (0x9d, 0x7c, 0xd8),
            success: (0x7f, 0xd8, 0x8f),
            error: (0xe0, 0x6c, 0x75),
            warning: (0xf5, 0xa7, 0x42),
            info: (0x56, 0xb6, 0xc2),
            text: (0xee, 0xee, 0xee),
            muted: (0x80, 0x80, 0x80),
            border: (0x48, 0x48, 0x48),
            border_active: (0x60, 0x60, 0x60),
            star_dim: (0x2c, 0x2c, 0x2c),
            star_bright: (0x40, 0x40, 0x40),
            bg_panel: (0x24, 0x22, 0x22),
            bg_selected: (0x3a, 0x35, 0x32),
            bg_element: (0x2e, 0x2b, 0x29),
            bg_menu: (0x2a, 0x27, 0x26),
            backdrop: (0x0a, 0x0a, 0x0c),
            base: (0x00, 0x00, 0x00),
            light: false,
            stars: true,
            square: false,
        },
        ThemeName::Nord => Theme {
            name: "nord",
            primary: (0x88, 0xc0, 0xd0),
            accent: (0x8f, 0xbc, 0xbb),
            success: (0xa3, 0xbe, 0x8c),
            error: (0xbf, 0x61, 0x6a),
            warning: (0xd0, 0x87, 0x70),
            info: (0x81, 0xa1, 0xc1),
            text: (0xec, 0xef, 0xf4),
            muted: (0x8b, 0x95, 0xa7),
            border: (0x43, 0x4c, 0x5e),
            border_active: (0x4c, 0x56, 0x6a),
            star_dim: (0x28, 0x2e, 0x39),
            star_bright: (0x39, 0x41, 0x50),
            bg_panel: (0x24, 0x2a, 0x33),
            bg_selected: (0x3b, 0x42, 0x52),
            bg_element: (0x2e, 0x35, 0x40),
            bg_menu: (0x2a, 0x30, 0x3a),
            backdrop: (0x0a, 0x0a, 0x0c),
            base: (0x00, 0x00, 0x00),
            light: false,
            stars: true,
            square: false,
        },
        ThemeName::Gruvbox => Theme {
            name: "gruvbox",
            primary: (0x83, 0xa5, 0x98),
            accent: (0x8e, 0xc0, 0x7c),
            success: (0xb8, 0xbb, 0x26),
            error: (0xfb, 0x49, 0x34),
            warning: (0xfe, 0x80, 0x19),
            info: (0xfa, 0xbd, 0x2f),
            text: (0xeb, 0xdb, 0xb2),
            muted: (0x92, 0x83, 0x74),
            border: (0x66, 0x5c, 0x54),
            border_active: (0x7c, 0x6f, 0x64),
            star_dim: (0x3b, 0x35, 0x30),
            star_bright: (0x55, 0x4d, 0x46),
            bg_panel: (0x28, 0x24, 0x21),
            bg_selected: (0x3f, 0x38, 0x33),
            bg_element: (0x33, 0x2d, 0x28),
            bg_menu: (0x2e, 0x29, 0x25),
            backdrop: (0x0a, 0x0a, 0x0c),
            base: (0x00, 0x00, 0x00),
            light: false,
            stars: true,
            square: false,
        },
        ThemeName::RosePine => Theme {
            name: "rosepine",
            primary: (0x9c, 0xcf, 0xd8),
            accent: (0xeb, 0xbc, 0xba),
            success: (0x31, 0x74, 0x8f),
            error: (0xeb, 0x6f, 0x92),
            warning: (0xf6, 0xc1, 0x77),
            info: (0xc4, 0xa7, 0xe7),
            text: (0xe0, 0xde, 0xf4),
            muted: (0x6e, 0x6a, 0x86),
            border: (0x40, 0x3d, 0x52),
            border_active: (0x52, 0x4f, 0x67),
            star_dim: (0x25, 0x24, 0x31),
            star_bright: (0x35, 0x33, 0x45),
            bg_panel: (0x1f, 0x1d, 0x28),
            bg_selected: (0x31, 0x2e, 0x40),
            bg_element: (0x2a, 0x26, 0x36),
            bg_menu: (0x25, 0x22, 0x2f),
            backdrop: (0x0a, 0x0a, 0x0c),
            base: (0x00, 0x00, 0x00),
            light: false,
            stars: true,
            square: false,
        },
        ThemeName::Tokyonight => Theme {
            name: "tokyonight",
            primary: (0x82, 0xaa, 0xff),
            accent: (0xc0, 0x99, 0xff),
            success: (0xc3, 0xe8, 0x8d),
            error: (0xff, 0x75, 0x7f),
            warning: (0xff, 0x96, 0x6c),
            info: (0x82, 0xaa, 0xff),
            text: (0xc8, 0xd3, 0xf5),
            muted: (0x82, 0x8b, 0xb8),
            border: (0x3b, 0x42, 0x64),
            border_active: (0x54, 0x5c, 0x7e),
            star_dim: (0x22, 0x27, 0x3b),
            star_bright: (0x31, 0x37, 0x53),
            bg_panel: (0x1b, 0x1f, 0x30),
            bg_selected: (0x2f, 0x33, 0x4a),
            bg_element: (0x25, 0x29, 0x3c),
            bg_menu: (0x21, 0x25, 0x36),
            backdrop: (0x0a, 0x0a, 0x0c),
            base: (0x00, 0x00, 0x00),
            light: false,
            stars: true,
            square: false,
        },
        ThemeName::Catppuccin => Theme {
            name: "catppuccin",
            primary: (0x89, 0xb4, 0xfa),
            accent: (0xf5, 0xc2, 0xe7),
            success: (0xa6, 0xe3, 0xa1),
            error: (0xf3, 0x8b, 0xa8),
            warning: (0xf9, 0xe2, 0xaf),
            info: (0x94, 0xe2, 0xd5),
            text: (0xcd, 0xd6, 0xf4),
            muted: (0x93, 0x99, 0xb2),
            border: (0x31, 0x32, 0x44),
            border_active: (0x45, 0x47, 0x5a),
            star_dim: (0x1d, 0x1e, 0x28),
            star_bright: (0x29, 0x2a, 0x39),
            bg_panel: (0x1e, 0x1e, 0x27),
            bg_selected: (0x31, 0x31, 0x40),
            bg_element: (0x28, 0x28, 0x34),
            bg_menu: (0x24, 0x24, 0x2d),
            backdrop: (0x06, 0x09, 0x11),
            base: (0x00, 0x00, 0x00),
            light: false,
            stars: true,
            square: false,
        },
        // 亮色主题：整屏亮底 + 深色文字 + 无纹理 —— 与暗色主题是两种
        // 观感而非换色。遮罩用浅灰（黑遮罩在亮底上是刺眼的洞）。
        ThemeName::Light => Theme {
            name: "light",
            primary: (0x4c, 0x60, 0xd4),
            accent: (0x9d, 0x5c, 0xd8),
            success: (0x1a, 0x7f, 0x37),
            error: (0xc6, 0x28, 0x28),
            warning: (0xb2, 0x6a, 0x00),
            info: (0x02, 0x77, 0xbd),
            text: (0x2a, 0x2c, 0x32),
            muted: (0x6e, 0x72, 0x7a),
            border: (0xd4, 0xd4, 0xd0),
            border_active: (0xb0, 0xb4, 0xc0),
            star_dim: (0xc0, 0xc0, 0xbc),
            star_bright: (0xcc, 0xcc, 0xc8),
            bg_panel: (0xf0, 0xf0, 0xee),
            bg_selected: (0xe2, 0xe6, 0xf6),
            bg_element: (0xea, 0xea, 0xe8),
            bg_menu: (0xe6, 0xe6, 0xe4),
            backdrop: (0xe6, 0xe6, 0xe4),
            base: (0xfa, 0xfa, 0xf8),
            light: true,
            stars: false,
            square: false,
        },
        // CRT 绿磷终端风：方角边框、无纹理、通体磷绿 —— 风格来自
        // 形状（方角）与单色系，而不是又一组和谐配色。
        ThemeName::Terminal => Theme {
            name: "terminal",
            primary: (0x53, 0xff, 0x9c),
            accent: (0xa8, 0xff, 0xc8),
            success: (0x53, 0xff, 0x9c),
            error: (0xff, 0x5c, 0x5c),
            warning: (0xe8, 0xd4, 0x5c),
            info: (0x5c, 0xd4, 0xe8),
            text: (0x2e, 0xe5, 0x74),
            muted: (0x1d, 0x9a, 0x52),
            border: (0x14, 0x66, 0x38),
            border_active: (0x1e, 0x8f, 0x50),
            star_dim: (0x0a, 0x2a, 0x16),
            star_bright: (0x12, 0x42, 0x24),
            bg_panel: (0x07, 0x1a, 0x0e),
            bg_selected: (0x0d, 0x2a, 0x18),
            bg_element: (0x0a, 0x20, 0x12),
            bg_menu: (0x09, 0x1c, 0x10),
            backdrop: (0x01, 0x05, 0x03),
            base: (0x03, 0x09, 0x05),
            light: false,
            stars: false,
            square: true,
        },
    }
}

/// 主题偏好在用户目录下的存储路径。
fn store_path() -> std::path::PathBuf {
    let home = std::env::var_os("NEO_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(std::path::PathBuf::from))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    home.join(".neo").join("theme")
}

/// 读取用户上次选的主题。任何失败都退回默认 —— 主题不该拦住启动。
pub fn load_preference() -> ThemeName {
    std::fs::read_to_string(store_path())
        .ok()
        .and_then(|s| ThemeName::parse(&s))
        .unwrap_or_default()
}

/// 记住用户选择。写失败不上报：主题偏好无关紧要，不该打断会话。
pub fn save_preference(name: ThemeName) {
    let path = store_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, name.as_str());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_is_listed_and_resolvable() {
        for n in ThemeName::all() {
            let t = get(n);
            assert_eq!(t.name, n.as_str(), "主题名与枚举必须一致");
        }
    }

    #[test]
    fn starfield_is_dimmer_than_the_border() {
        // 星场是背景：两档都必须比边框暗，否则会跟正文抢注意力。
        // 无纹理主题（light/terminal）不渲染星场，亮度次序不适用。
        for n in ThemeName::all() {
            let t = get(n);
            if !t.stars {
                continue;
            }
            let lum = |c: (u8, u8, u8)| c.0 as u32 + c.1 as u32 + c.2 as u32;
            assert!(
                lum(t.star_bright) < lum(t.border),
                "{} 的亮星比边框还亮，会喧宾夺主",
                t.name
            );
            assert!(
                lum(t.star_dim) < lum(t.star_bright),
                "{} 的星场两档亮度必须拉开",
                t.name
            );
        }
    }

    #[test]
    fn text_contrasts_more_than_muted_in_every_theme() {
        // 正文与次要文字的层次不能反，否则"哪个是重点"会看错。
        // "层次"以**与底色的对比**度量：暗色主题比谁更亮，亮色主题
        // 比谁更深 —— 对亮色主题断言"更亮"等于允许次要不清晰。
        for n in ThemeName::all() {
            let t = get(n);
            let lum = |c: (u8, u8, u8)| c.0 as u32 + c.1 as u32 + c.2 as u32;
            if t.light {
                assert!(lum(t.text) < lum(t.muted), "{} 亮色正文应比次要文字更深", t.name);
            } else {
                assert!(lum(t.text) > lum(t.muted), "{} 正文应比次要文字亮", t.name);
            }
        }
    }

    #[test]
    fn parse_accepts_aliases_and_rejects_junk() {
        assert_eq!(ThemeName::parse("default"), Some(ThemeName::Neo));
        assert_eq!(ThemeName::parse("Rose-Pine"), Some(ThemeName::RosePine));
        assert_eq!(ThemeName::parse("  NORD "), Some(ThemeName::Nord));
        assert_eq!(ThemeName::parse("不存在"), None);
    }

    #[test]
    fn the_default_theme_is_neos_own_purple() {
        // 默认主题必须是 NEO 自己的紫，而不是参考主题的橙。
        // 判据用色相族而不是硬编码三原色值：紫 = 蓝/红高、绿低。
        assert_eq!(ThemeName::default(), ThemeName::Neo);
        let t = get(ThemeName::default());
        let (r, g, b) = t.primary;
        assert!(
            b as u16 > g as u16 + 60 && r as u16 > g as u16 + 20,
            "默认主色应是紫（蓝红高、绿低），实际 #{r:02x}{g:02x}{b:02x}"
        );
    }

    #[test]
    fn opencode_reference_theme_is_still_reachable() {
        // 参考主题保留，供对照；但它不再是默认
        assert_ne!(ThemeName::default(), ThemeName::OpenCode);
        assert!(ThemeName::all().contains(&ThemeName::OpenCode));
        assert!(ThemeName::parse("opencode").is_some());
    }

    #[test]
    fn next_cycles_through_all_and_wraps() {
        let mut seen = vec![ThemeName::default()];
        let mut cur = ThemeName::default();
        for _ in 0..ThemeName::all().len() - 1 {
            cur = cur.next();
            assert!(!seen.contains(&cur), "循环中不该重复");
            seen.push(cur);
        }
        assert_eq!(cur.next(), ThemeName::default(), "应回到起点");
    }
}
