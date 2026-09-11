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
}

impl Default for ThemeName {
    fn default() -> Self {
        Self::Neo
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
            _ => return None,
        })
    }

    /// 按名字顺序列出（切换键遍历用；顺序稳定，用户能习惯）。
    pub fn all() -> [ThemeName; 7] {
        [
            Self::Neo,
            Self::OpenCode,
            Self::Nord,
            Self::Gruvbox,
            Self::RosePine,
            Self::Tokyonight,
            Self::Catppuccin,
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
        // 星场是背景：两档都必须比边框暗，否则会跟正文抢注意力
        for n in ThemeName::all() {
            let t = get(n);
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
    fn text_is_brighter_than_muted_in_every_theme() {
        // 正文与次要文字的层次不能反，否则"哪个是重点"会看错
        for n in ThemeName::all() {
            let t = get(n);
            let lum = |c: (u8, u8, u8)| c.0 as u32 + c.1 as u32 + c.2 as u32;
            assert!(lum(t.text) > lum(t.muted), "{} 正文应比次要文字亮", t.name);
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
