//! 外观（背景图与 Logo 样式）—— 纯装饰，可切换、可持久化
//!
//! # 为什么背景"图片"在终端里是字符画
//!
//! 终端没有位图通道（除少数协议的图像扩展，但支持面窄且不可移植）。
//! 所谓"背景图片"在本项目里是**确定性的字符纹理**：星场、点阵、雨、
//! 网格等。它们与原生的位图背景在观感上不是一回事，命名上如实叫
//! `Background` 而不是"图片"，避免误导。
//!
//! # 铁律：装饰不得进入语义
//!
//! 这些纹理不进 `Fact`、不影响 T6 宿主等价、`NO_COLOR` 下自动消失。
//! 它们只填"从未被内容写入"的格子（见 `Grid::fill_*`），因此永远
//! 不会盖住正文 —— 这是它们与内容的唯一关系。
//!
//! # 内置位图（"图片"）的做法
//!
//! 对真正想放一张图的用户，务实做法是**把图片转成半角字符的灰度图**
//! 在启动时读入。那需要一个图像解码器（引依赖）或调用外部命令；
//! 本模块不引入这两者，而是提供一个**字符画文件**通道：
//! 用户给一个 UTF-8 文本文件，每行即为背景的一行（支持 `NEO_TUI_BG_FILE`）。
//! 这样"自定义背景"是可达的，且零依赖 —— 边界写在文档里。

/// 背景纹理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Background {
    /// 星场（默认）：稀疏点阵
    Stars,
    /// 点阵网格：规律排列的细点
    Dots,
    /// 斜纹：错开排列的斜线
    Diagonal,
    /// 纯色（无纹理）
    None,
}

impl Default for Background {
    fn default() -> Self { Self::Stars }
}

impl Background {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stars => "stars",
            Self::Dots => "dots",
            Self::Diagonal => "diagonal",
            Self::None => "none",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "stars" | "star" | "default" => Self::Stars,
            "dots" | "grid" => Self::Dots,
            "diagonal" | "lines" => Self::Diagonal,
            "none" | "off" | "plain" | "solid" => Self::None,
            _ => return None,
        })
    }

    pub fn all() -> [Background; 4] {
        [Self::Stars, Self::Dots, Self::Diagonal, Self::None]
    }

    pub fn next(self) -> Self {
        let all = Self::all();
        let i = all.iter().position(|x| *x == self).unwrap_or(0);
        all[(i + 1) % all.len()]
    }

    /// 该纹理在某格显示什么。
    ///
    /// **确定性**：同一 `(row, col, cols)` 永远同一结果 —— TUI 每帧重画，
    /// 随机纹理会让画面闪成噪点（星场那节的教训）。
    pub fn cell(self, row: usize, col: usize, cols: usize) -> Option<(char, Brightness)> {
        match self {
            Self::None => None,
            Self::Stars => stars_like(row, col, cols, 1000, 9, '·', '+'),
            Self::Dots => {
                // 规律网格：每 3 行 6 列一个点。规律意味着**不闪**
                //（位置固定），也意味着更安静 —— 适合长时间盯着看。
                if row % 3 == 1 && col % 6 == 2 {
                    Some(('·', Brightness::Dim))
                } else {
                    None
                }
            }
            Self::Diagonal => {
                // 斜纹：相位 1/17 且**每 2 行才考虑一次** —— 1/7 太密（14%），
                // 背景会盖过正文的观感。目标是 2%~5% 这个"看得见但不抢眼"区间。
                if row % 2 == 0 && (row + col * 2) % 17 == 0 {
                    Some(('\\', Brightness::Dim))
                } else {
                    None
                }
            }
        }
    }
}

/// 亮度档（决定用哪一档色调）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Brightness {
    Dim,
    Bright,
}

/// 与 `stars::cell` 同族的确定性哈希（保证不同纹理间也有差异）。
fn stars_like(
    row: usize,
    col: usize,
    cols: usize,
    modulo: u64,
    hot: u64,
    dim_ch: char,
    bright_ch: char,
) -> Option<(char, Brightness)> {
    let mut h = (row as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (col as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (cols as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    // 用 guard 而非范围模式：`modulo` / `hot` 是运行时参数，
    // 范围模式只能用于常量。
    let v = h % modulo;
    if v <= 1 {
        Some((bright_ch, Brightness::Bright))
    } else if v <= hot {
        Some((dim_ch, Brightness::Dim))
    } else {
        None
    }
}

/// Logo 样式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogoStyle {
    /// 大字号（ANSI Shadow，6 行）+ 渐变（默认）
    Large,
    /// 小字号（3 行，block 字符）
    Small,
    /// 极简：只有一行 `NEO`
    Minimal,
    /// 不显示 Logo（首屏只留信息与输入框）
    Hidden,
}

impl Default for LogoStyle {
    fn default() -> Self { Self::Large }
}

impl LogoStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Large => "large",
            Self::Small => "small",
            Self::Minimal => "minimal",
            Self::Hidden => "hidden",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "large" | "big" | "default" => Self::Large,
            "small" | "compact" => Self::Small,
            "minimal" | "tiny" | "one-line" => Self::Minimal,
            "hidden" | "none" | "off" => Self::Hidden,
            _ => return None,
        })
    }

    pub fn all() -> [LogoStyle; 4] {
        [Self::Large, Self::Small, Self::Minimal, Self::Hidden]
    }

    pub fn next(self) -> Self {
        let all = Self::all();
        let i = all.iter().position(|x| *x == self).unwrap_or(0);
        all[(i + 1) % all.len()]
    }

    /// 该样式需要多少行（`Large` 是 6 行，用于首屏高度预算）。
    pub fn rows(self) -> usize {
        match self {
            Self::Large => 6,
            Self::Small => 3,
            Self::Minimal => 1,
            Self::Hidden => 0,
        }
    }
}

/// 小字号词标（3 行，用 block 字符，宽度 21）。
///
/// 每行必须等宽 —— 不等宽在终端里立刻看出错位（大词标那节的教训）。
pub const LOGO_SMALL: [&str; 3] = [
    "█▀▀█ █▀▀▀ █▀▀█",
    "█  █ █▀▀▀ █  █",
    "▀▀▀▀ ▀▀▀▀ ▀▀▀▀",
];

/// 外观偏好（可持久化）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Appearance {
    pub background: Background,
    pub logo: LogoStyle,
}

impl Default for Appearance {
    fn default() -> Self {
        Self { background: Background::Stars, logo: LogoStyle::Large }
    }
}

/// 偏好的存储路径。
fn store_path() -> std::path::PathBuf {
    let home = std::env::var_os("NEO_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(std::path::PathBuf::from))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    home.join(".neo").join("appearance")
}

/// 读取偏好。格式：两行 `background` / `logo`；任何失败退回默认值 ——
/// 外观不该拦住启动。
pub fn load_preference() -> Appearance {
    let Ok(raw) = std::fs::read_to_string(store_path()) else {
        return Appearance::default();
    };
    let mut a = Appearance::default();
    for line in raw.lines() {
        if let Some(v) = line.strip_prefix("background=") {
            if let Some(b) = Background::parse(v) {
                a.background = b;
            }
        } else if let Some(v) = line.strip_prefix("logo=") {
            if let Some(l) = LogoStyle::parse(v) {
                a.logo = l;
            }
        }
    }
    a
}

/// 记住偏好。写失败静默忽略（外观无关紧要，不该打断会话）。
pub fn save_preference(ap: Appearance) {
    let path = store_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(
        path,
        format!("background={}\nlogo={}\n", ap.background.as_str(), ap.logo.as_str()),
    );
}

/// 读取自定义背景（字符画文件）。
///
/// 用户可用 `NEO_TUI_BG_FILE` 指定一个 UTF-8 文本文件，每行作为背景的一行。
/// 这给"放一张自己的图"留了通道，且**零依赖**：不做图像解码，
/// 由用户自行把图转成字符画（工具链成熟，也避免我们引一个解码器）。
///
/// 内存有界：最多读 `max_rows × max_cols` 个字符，超出的行截掉。
pub fn load_custom_background(
    path: &std::path::Path,
    max_rows: usize,
    max_cols: usize,
) -> Option<Vec<String>> {
    let raw = std::fs::read_to_string(path).ok()?;
    let lines: Vec<String> = raw
        .lines()
        .take(max_rows)
        .map(|l| l.chars().take(max_cols).collect())
        .collect();
    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_background_is_listed_and_parsable() {
        for b in Background::all() {
            assert_eq!(Background::parse(b.as_str()), Some(b), "{b:?} 应可往返解析");
        }
        assert_eq!(Background::parse("default"), Some(Background::Stars));
        assert_eq!(Background::parse("  OFF "), Some(Background::None));
        assert_eq!(Background::parse("不存在"), None);
    }

    #[test]
    fn every_logo_style_is_listed_and_parsable() {
        for l in LogoStyle::all() {
            assert_eq!(LogoStyle::parse(l.as_str()), Some(l), "{l:?} 应可往返解析");
        }
        assert_eq!(LogoStyle::parse("default"), Some(LogoStyle::Large));
        assert_eq!(LogoStyle::parse("nope"), None);
    }

    #[test]
    fn textures_are_deterministic() {
        // 关键：TUI 每帧重画，随机纹理会让画面闪成噪点
        for b in Background::all() {
            for (r, c) in [(0, 0), (3, 17), (40, 199), (7, 42)] {
                assert_eq!(b.cell(r, c, 200), b.cell(r, c, 200), "{b:?} 在 ({r},{c}) 不稳定");
            }
        }
    }

    #[test]
    fn none_background_draws_nothing() {
        for (r, c) in [(0, 0), (5, 5), (10, 20)] {
            assert_eq!(Background::None.cell(r, c, 80), None);
        }
    }

    #[test]
    fn dots_texture_is_regular_not_noisy() {
        // 网格纹理的价值就在"规律"：同一列上的点应周期性出现
        let col = 2usize;
        let rows_with_dot: Vec<usize> =
            (0..24).filter(|r| Background::Dots.cell(*r, col, 80).is_some()).collect();
        assert_eq!(rows_with_dot, vec![1, 4, 7, 10, 13, 16, 19, 22], "应为每 3 行一个点");
    }

    #[test]
    fn all_textures_use_narrow_characters_only() {
        // 宽字符会让整行右移一列 —— 背景错一列，整屏排版就歪了
        for b in Background::all() {
            for r in 0..60 {
                for c in 0..120 {
                    if let Some((ch, _)) = b.cell(r, c, 120) {
                        assert_eq!(
                            crate::width::char_width(ch),
                            1,
                            "{b:?} 在 ({r},{c}) 用了宽字符 {ch:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn texture_density_is_low_but_nonzero() {
        // 太密 = 噪点；全空 = 白做
        for b in [Background::Stars, Background::Dots, Background::Diagonal] {
            let (cols, rows) = (160usize, 50usize);
            let n = (0..rows)
                .flat_map(|r| (0..cols).map(move |c| (r, c)))
                .filter(|(r, c)| b.cell(*r, *c, cols).is_some())
                .count();
            let ratio = n as f64 / (cols * rows) as f64;
            assert!(ratio > 0.001 && ratio < 0.08, "{b:?} 密度 {ratio:.4} 不合理");
        }
    }

    #[test]
    fn logo_small_rows_are_equal_width() {
        // 手改小词标最容易某行差一个字符 —— 终端里立刻看出错位
        let ws: Vec<usize> = LOGO_SMALL.iter().map(|r| r.chars().count()).collect();
        assert!(ws.windows(2).all(|w| w[0] == w[1]), "小词标各行必须等宽，实际 {ws:?}");
    }

    #[test]
    fn logo_style_rows_match_actual_art() {
        assert_eq!(LogoStyle::Large.rows(), 6);
        assert_eq!(LogoStyle::Small.rows(), LOGO_SMALL.len());
        assert_eq!(LogoStyle::Minimal.rows(), 1);
        assert_eq!(LogoStyle::Hidden.rows(), 0);
    }

    #[test]
    fn cycles_wrap_through_all_styles() {
        let mut seen = vec![Background::default()];
        let mut cur = Background::default();
        for _ in 0..Background::all().len() - 1 {
            cur = cur.next();
            assert!(!seen.contains(&cur));
            seen.push(cur);
        }
        assert_eq!(cur.next(), Background::default());

        let mut seen = vec![LogoStyle::default()];
        let mut cur = LogoStyle::default();
        for _ in 0..LogoStyle::all().len() - 1 {
            cur = cur.next();
            assert!(!seen.contains(&cur));
            seen.push(cur);
        }
        assert_eq!(cur.next(), LogoStyle::default());
    }

    #[test]
    fn custom_background_is_bounded() {
        // 内存有界：超出行数/列数的部分必须截掉
        let dir = std::env::temp_dir().join(format!("neo-bg-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("bg.txt");
        std::fs::write(&f, (0..100).map(|_| format!("{}\n", "x".repeat(300))).collect::<String>())
            .unwrap();
        let bg = load_custom_background(&f, 5, 10).expect("应有内容");
        assert_eq!(bg.len(), 5, "行数应被截到 5");
        for l in &bg {
            assert_eq!(l.chars().count(), 10, "列数应被截到 10");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_custom_background_is_none_not_a_panic() {
        let p = std::path::Path::new("/definitely/not/here/neo-bg.txt");
        assert!(load_custom_background(p, 10, 10).is_none());
    }
}

// ══════════════════════════════════════════════════════════════════════
// 显示偏好（工具输出展开 / 推理显隐）
// ══════════════════════════════════════════════════════════════════════
//
// 与外观分开存一个文件：两者的语义不同（一个管"长什么样"，
// 一个管"显示多少信息"），混在一个文件里将来加字段容易互相覆盖。
//
// **为什么必须持久化**：用户明确反馈"我选了显示思考过程，怎么又要重新设置"——
// 之前这两个开关只在内存里，重启 TUI 就回到默认（都隐藏）。
// 主题与背景都持久化了，这两个没有，是不一致。

fn display_store_path() -> std::path::PathBuf {
    let home = std::env::var_os("NEO_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(std::path::PathBuf::from))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    home.join(".neo").join("display")
}

/// 读取显示偏好。失败一律退回默认 —— 偏好不该拦住启动。
pub fn load_display_pref() -> DisplayPref {
    let Ok(raw) = std::fs::read_to_string(display_store_path()) else {
        return DisplayPref::default();
    };
    let mut d = DisplayPref::default();
    for line in raw.lines() {
        match line.split_once('=') {
            Some(("details", v)) => d.details = v.trim() == "on",
            Some(("thinking", v)) => d.thinking = v.trim() == "on",
            _ => {}
        }
    }
    d
}

pub fn save_display_pref(d: DisplayPref) {
    let path = display_store_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(
        path,
        format!(
            "details={}\nthinking={}\n",
            if d.details { "on" } else { "off" },
            if d.thinking { "on" } else { "off" },
        ),
    );
}

/// 显示偏好（对应 TUI 的 `ToolDisplay`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DisplayPref {
    pub details: bool,
    pub thinking: bool,
}
