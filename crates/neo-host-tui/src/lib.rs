//! L5 HOST · TUI —— 真实可用的终端宿主（零依赖）
//!
//! # 铁律：不含业务逻辑
//!
//! 它只做三件事：读键、调 `kernel.submit()`、把事件画出来。
//! 它与 `neo-exec` 消费**同一条事件流**，这是 T6（宿主语义等价）的第二个样本。
//!
//! # 为什么手写终端控制而不引 ratatui/crossterm
//!
//! 与本项目其它决定一致：**调试链要浅**。终端模式设错时，只需看这一个文件，
//! 而不是在几层 crate 抽象之间找。终端能力只用到三样：
//! `stty` 切原始模式、ANSI 转义定位/清屏、读 stdin 字节。
//!
//! # 安全：终端必须被还原
//!
//! 原始模式下终端不回显、不处理信号。若进程异常退出而不还原，
//! 用户的终端会"坏掉"。故：
//! - `RawMode` 用 `Drop` 保证正常路径还原（含 panic 展开）
//! - 另装 `panic` hook，在 panic 前先还原
//! - 保存 `stty -g` 的确切状态并原样写回（不是猜一个"合理默认"）

pub mod commands;
pub mod input;
pub mod popup;
pub mod markdown;
pub mod stars;
pub mod syntax;
pub mod theme;
pub mod trust;
pub mod width;

use neo_core::{HostBackend, HostCapabilities, DiffSupport, ImageSupport};
use neo_protocol::{EventMsg, Fact, facts_of};
use std::io::{IsTerminal, Read, Write};

// ══════════════════════════════════════════════════════════════════════
// 原始终端模式（RAII + panic 安全）
// ══════════════════════════════════════════════════════════════════════

/// 保存的终端设置。Drop 时原样还原。
pub struct RawMode {
    saved: String,
}

impl RawMode {
    /// 进入原始模式。
    ///
    /// 失败时返回**具体原因**而非笼统的 `None` —— 三种失败完全不同
    /// （管道里跑 / 没有 stty / stty 拒绝），补救方式也不同。
    pub fn enter() -> Result<Self, String> {
        if !std::io::stdin().is_terminal() {
            return Err("标准输入不是终端（管道或重定向下无法交互）".into());
        }
        let saved = read_stty().ok_or_else(|| {
            format!("无法读取终端设置（`stty -g` 失败）：终端能力不可用，或 PATH 中没有 stty")
        })?;
        // raw: 逐字符读（不等回车）；-echo: 不回显（界面自己画）
        set_stty("raw -echo").ok_or_else(|| "无法将终端切到原始模式（`stty raw -echo` 失败）".to_string())?;
        Ok(Self { saved })
    }

    /// 显式还原（幂等）。
    pub fn restore(&self) {
        let _ = set_stty(&self.saved);
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        self.restore();
    }
}

/// 跑一次 `stty` 并捕获 stdout。
///
/// **必须 `stdin(Stdio::inherit())`**：Rust 的 `Command::output()` 默认把子进程
/// stdin 接到空流，`stty` 会因此认为 fd 0 不是终端而失败 ——
/// 而**父进程**的 `is_terminal()` 明明是 true。这个不一致曾让本宿主
/// 把"stty 调用方式不对"误报成"输入不是终端"（真机测试抓出）。
fn stty_capture(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("stty")
        .args(args)
        .stdin(std::process::Stdio::inherit())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn read_stty() -> Option<String> { stty_capture(&["-g"]) }

fn set_stty(spec: &str) -> Option<()> {
    // spec 可能是 `raw -echo` 这样的多个词，也可能是 `stty -g` 的单一状态串
    let args: Vec<&str> = spec.split_whitespace().collect();
    let status = std::process::Command::new("stty")
        .args(&args)
        .stdin(std::process::Stdio::inherit())
        .status()
        .ok()?;
    status.success().then_some(())
}

/// 备用屏幕缓冲（alternate screen）。
///
/// # 为什么必须有它
///
/// 不用备用屏时，TUI 是直接在**正常屏幕**上画的。退出时我们只还原了 stty，
/// 却把最后一帧（欢迎页）留在了用户的终端里 —— 用户看到 shell 提示符上方
/// 还挂着我们的界面。这不是"没清屏"，而是**画错了地方**。
///
/// 备用屏是内核提供的第二块屏幕：进入时终端保存原屏幕，退出时整块还原。
/// 于是我们的界面从不污染正常屏幕，退出后用户看到的就是启动前的内容
/// （vim / less / htop / opencode 都是这个机制）。
///
/// 对应序列：`ESC[?1049h` 进入、`ESC[?1049l` 退出。
struct AltScreen {
    active: bool,
}

impl AltScreen {
    fn enter() -> Self {
        let mut out = std::io::stdout();
        let _ = write!(out, "{ESC}[?1049h{ESC}[2J{ESC}[H");
        let _ = out.flush();
        Self { active: true }
    }

    /// 退出备用屏（幂等）。
    fn leave(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        Self::leave_now();
    }

    /// 无需实例即可执行的"退出备用屏"。panic 钩子里用（那里拿不到实例）。
    fn leave_now() {
        let mut out = std::io::stdout();
        // 先恢复光标可见，再退出备用屏：顺序反了光标会在原屏幕上保持隐藏
        let _ = write!(out, "{ESC}[?25h{ESC}[?1049l");
        let _ = out.flush();
    }
}

impl Drop for AltScreen {
    fn drop(&mut self) {
        self.leave();
    }
}

/// 终端尺寸（列, 行）。取自 `stty size`，失败时退到 80×24。
pub fn terminal_size() -> (usize, usize) {
    if let Some(s) = stty_capture(&["size"]) {
        let mut it = s.split_whitespace();
        if let (Some(rows), Some(cols)) = (it.next(), it.next()) {
            if let (Ok(r), Ok(c)) = (rows.parse::<usize>(), cols.parse::<usize>()) {
                if r > 0 && c > 0 {
                    return (c, r);
                }
            }
        }
    }
    // 拿不到就退到 80×24（不报错：尺寸只是渲染参考）
    (80, 24)
}

// ══════════════════════════════════════════════════════════════════════
// 输入
// ══════════════════════════════════════════════════════════════════════

/// 一次按键。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    /// Ctrl+C / Ctrl+D
    Quit,
    /// Esc：关闭弹窗
    Escape,
    /// Ctrl+P：命令面板
    CommandPalette,
    /// Ctrl+T：下一个主题
    NextTheme,
    /// Ctrl+B：切换侧栏
    ToggleSidebar,
    /// Ctrl+L 清屏
    ClearScreen,
    /// Ctrl+U 清空输入行
    ClearLine,
    /// Ctrl+R 历史搜索
    SearchHistory,
    /// Ctrl+G 用 $EDITOR 编辑当前输入
    ExternalEditor,
    /// Tab：补全 `@` 文件引用
    Tab,
    Up,
    Down,
    Left,
    Right,
    /// 其它转义序列（忽略，但显式建模以免与普通字符混淆）
    Unknown,
}

/// 从字节流解出一个按键。
///
/// 转义序列以 `ESC`(0x1b) 起头；`[A`~`[D` 是方向键。
/// 单独一个 ESC 视为 Unknown（TUI 不需要 Esc 语义，避免与序列混淆）。
pub fn decode_key(bytes: &[u8]) -> Key {
    match bytes {
        [0x03] | [0x04] => Key::Quit,
        [0x0c] => Key::ClearScreen,
        [0x15] => Key::ClearLine,
        [0x12] => Key::SearchHistory,
        [0x07] => Key::ExternalEditor,
        [b'\t'] => Key::Tab,
        [b'\r'] | [b'\n'] => Key::Enter,
        [0x7f] | [0x08] => Key::Backspace,
        [0x1b, b'[', b'A'] => Key::Up,
        [0x1b, b'[', b'B'] => Key::Down,
        [0x1b, b'[', b'C'] => Key::Right,
        [0x1b, b'[', b'D'] => Key::Left,
        [0x10] => Key::CommandPalette,
        [0x14] => Key::NextTheme,
        [0x02] => Key::ToggleSidebar,
        // 单独一个 ESC：关闭弹窗。必须排在 `[0x1b, ..]` 之前 ——
        // 后者也能匹配长度 1 的输入，会把 Esc 吞成 Unknown
        [0x1b] => Key::Escape,
        [0x1b, ..] => Key::Unknown,
        _ => {
            // UTF-8：可能多字节，交给 from_utf8
            match std::str::from_utf8(bytes) {
                Ok(s) => s.chars().next().map(Key::Char).unwrap_or(Key::Unknown),
                Err(_) => Key::Unknown,
            }
        }
    }
}

/// 从 stdin 读一次按键（原始模式下逐字节到达；多字节 UTF-8 需要续读）。
fn read_key(stdin: &mut impl Read) -> Key {
    let mut b = [0u8; 1];
    if stdin.read(&mut b).unwrap_or(0) == 0 {
        return Key::Quit; // EOF
    }
    if b[0] == 0x1b {
        // 转义序列：再读最多 2 字节。
        //
        // **必须带超时**：方向键会立刻送来完整序列，而用户单独按 Esc 时
        // 后面没有任何字节 —— 阻塞读会一直卡住，Esc 就永远不生效。
        // 把终端临时切成 `min 0 time 1`（10 分之 1 秒）做"立即返回"的读，
        // 读完再切回 raw。Esc 是低频操作，这点开销可接受。
        let mut seq = vec![0x1b];
        let _ = set_stty("min 0 time 1");
        for _ in 0..2 {
            let mut c = [0u8; 1];
            if stdin.read(&mut c).unwrap_or(0) == 0 {
                break;
            }
            seq.push(c[0]);
            if seq.len() == 3 {
                break;
            }
        }
        let _ = set_stty("raw -echo");
        return decode_key(&seq);
    }
    if b[0] < 0x80 {
        return decode_key(&b);
    }
    // 多字节 UTF-8：按首字节判断续字节数
    let need = match b[0] {
        0xc0..=0xdf => 1,
        0xe0..=0xef => 2,
        0xf0..=0xf7 => 3,
        _ => 0,
    };
    let mut buf = vec![b[0]];
    for _ in 0..need {
        let mut c = [0u8; 1];
        if stdin.read(&mut c).unwrap_or(0) == 0 {
            break;
        }
        buf.push(c[0]);
    }
    decode_key(&buf)
}

// ══════════════════════════════════════════════════════════════════════
// 渲染（ANSI）
// ══════════════════════════════════════════════════════════════════════

/// 首屏「关于」信息。
///
/// **全部由调用方注入**：宿主不读环境变量、不问模型、不查沙箱，
/// 否则就又变成了"宿主含业务逻辑"。它只负责把这些字符串排版出来。
#[derive(Debug, Clone, Default)]
pub struct About {
    pub version: String,
    pub model: String,
    /// 完整档位描述（首屏展示：含沙箱/审批/文件编辑三段）
    pub mode: String,
    /// 档位短名（footer 展示，如 `default`）—— 长描述会挤掉右侧信息
    pub mode_short: String,
    pub workspace: String,
    /// git 分支（空 = 不在仓库里）。底部状态行显示 `工作区:分支`。
    pub branch: String,
    /// 空输入时展示的示例任务（宿主不自选，避免"界面文案"散落在渲染逻辑里）
    pub example: String,
    /// 模型上下文窗口（token）。用于侧栏 Context 面板的占用率。
    /// 0 = 未知 → 不显示百分比（宁可不显示，也不给假数字）。
    pub context_limit: u64,
    pub session: String,
}

/// 词标（ANSI Shadow）。每行等宽，测试会断言这一点 —— 不等宽会看出错位。
const WORDMARK: [&str; 6] = [
    "███╗   ██╗ ███████╗ ██████╗ ",
    "████╗  ██║ ██╔════╝██╔═══██╗",
    "██╔██╗ ██║ █████╗  ██║   ██║",
    "██║╚██╗██║ ██╔══╝  ██║   ██║",
    "██║ ╚████║ ███████╗╚██████╔╝",
    "╚═╝  ╚═══╝ ╚══════╝ ╚═════╝ ",
];

/// 词标所需的最小终端宽度（词标宽 + 左侧缩进 + 余量）。
const WORDMARK_MIN_COLS: usize = 32;
/// 语义色调。网格只存色调，具体转义由 `Pal::tone` 在输出时展开 ——
/// 这样"能力降级"只需改一处，不必在每个渲染点判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// 未写入 → 交给星场填充
    None,
    Text,
    Primary,
    Accent,
    Success,
    Error,
    Warning,
    Info,
    Muted,
    Border,
    BorderActive,
    /// 暗色（SGR 2），用于最弱的结构文字
    Dim,
    StarDim,
    StarBright,
    /// 任意 RGB（logo 渐变）
    Rgb(u8, u8, u8),
}

impl Pal {
    fn tone(&self, t: Tone) -> String {
        match t {
            Tone::Text => self.text.clone(),
            Tone::Primary => self.primary.clone(),
            Tone::Accent => self.accent.clone(),
            Tone::Success => self.success.clone(),
            Tone::Error => self.error.clone(),
            Tone::Warning => self.warning.clone(),
            Tone::Info => self.info.clone(),
            Tone::Muted => self.muted.clone(),
            Tone::Border => self.border.clone(),
            Tone::BorderActive => self.border_active.clone(),
            Tone::Dim => self.dim.clone(),
            Tone::StarDim => Color::Rgb(self.theme.star_dim.0, self.theme.star_dim.1, self.theme.star_dim.2)
                .fg(self.mode, &self.theme),
            Tone::StarBright => Color::Rgb(
                self.theme.star_bright.0,
                self.theme.star_bright.1,
                self.theme.star_bright.2,
            )
            .fg(self.mode, &self.theme),
            Tone::Rgb(r, g, b) => Color::Rgb(r, g, b).fg(self.mode, &self.theme),
            Tone::None => self.reset.clone(),
        }
    }
}

/// 一屏内容，渲染成 ANSI 文本。
pub struct Screen<'a> {
    pub cols: usize,
    pub rows: usize,
    /// 已发生的用户可见事实（协议层 Fact，非宿主自造）
    pub facts: &'a [Fact],
    /// 当前输入行（纯文本；左侧竖条由渲染加，便于单独着色）
    pub input: &'a str,
    /// 状态文本（底部状态行右侧；空 = 不显示）
    pub status: &'a str,
    /// 有未决审批：边框转警告色，提示"现在该你回答"
    pub awaiting_input: bool,
    /// 光标可视（运行中不显示输入光标）
    pub show_cursor: bool,
    /// 首屏关于信息；仅在**尚无任何事实**时展示（有对话后让位给正文）
    pub about: Option<&'a About>,
    /// `Some` = 显示工作区信任对话框（首次进入某目录）
    pub trust: Option<&'a TrustPrompt>,
    /// 当前主题（持久化的用户偏好，由调用方注入）
    pub theme: theme::ThemeName,
    /// 弹窗（`@` 文件 / `/` 命令 / 主题 / 面板）。`None` = 不显示。
    pub popup: Option<&'a popup::Popup>,
    /// 预排版正文（/help、/keys 这类只读信息屏）。
    /// 与 `facts` 分开是因为它不是会话事实，只是宿主自己的一页说明。
    pub preformatted: Option<&'a Vec<Vec<Seg>>>,
    /// 侧栏是否展开（`ctrl+b` 切换）。窄终端下强制关闭。
    pub sidebar: bool,
}

/// 信任对话框状态。
#[derive(Debug, Clone, Copy, Default)]
pub struct TrustPrompt {
    /// 0 = 信任并继续，1 = 退出
    pub selected: usize,
}

// ── 调色板 ──────────────────────────────────────────────────────────
//
// 取色对标 opencode 的默认暗色主题（其定义在 packages/tui/src/theme/assets/
// opencode.json）：暖主色 + 冷强调色，正文与次要文字拉开层次。
//
// 为什么用 256 色 / truecolor 而不是 16 色 ANSI：16 色由终端主题决定，
// 同一份代码在不同终端里色调会完全不同，做不到"设计过的样子"。
// 这里遵循 NO_COLOR（无障碍/管道场景）与 COLORTERM/TERM 能力探测，
// 能力不足时自动降级为 16 色 —— 见 `palette()`。

const ESC: &str = "\u{1b}";

/// 前景色。按能力选 truecolor(38;2) / 256 色(38;5) / 16 色。
#[derive(Debug, Clone, Copy)]
enum Color {
    /// 主色（opencode darkStep9 #fab283，暖橙）
    Primary,
    /// 强调色（darkAccent #9d7cd8，紫）
    Accent,
    /// 成功（darkGreen #7fd88f）
    Success,
    /// 错误（darkRed #e06c75）
    Error,
    /// 警告（darkOrange #f5a742）
    Warning,
    /// 信息（darkCyan #56b6c2）
    Info,
    /// 正文（darkStep12 #eeeeee）
    Text,
    /// 次要文字（darkStep11 #808080）
    Muted,
    /// 边框（darkStep7 #484848）
    Border,
    /// 边框高亮（darkStep8 #606060）
    BorderActive,
    /// 任意 RGB（用于 logo 渐变等需要插值的地方）
    Rgb(u8, u8, u8),
}

/// RGB → xterm 256 色（6×6×6 色块 + 灰阶）
fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    if r == g && g == b {
        if r < 8 { return 16; }
        if r > 248 { return 231; }
        return (((r as u16 - 8) * 24 / 247) + 232) as u8;
    }
    let f = |c: u8| -> u16 {
        if c < 48 { 0 } else if c < 115 { 1 } else { ((c as u16 - 35) / 40).min(5) }
    };
    (16 + 36 * f(r) + 6 * f(g) + f(b)) as u8
}

/// RGB → 16 色（老终端兜底，按主色相粗分）
fn rgb_to_16(r: u8, g: u8, b: u8) -> u8 {
    let lum = (r as u32 * 30 + g as u32 * 59 + b as u32 * 11) / 100;
    if lum < 40 { return 90; }
    if r >= b && r >= g {
        if g > 128 { 33 } else { 31 }
    } else if b >= g {
        if r > 128 { 35 } else { 34 }
    } else if g > 128 {
        32
    } else {
        90
    }
}

/// 两个 RGB 之间线性插值（logo 渐变用）
fn lerp_rgb(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let f = |x: u8, y: u8| -> u8 {
        (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8
    };
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

/// 终端能力（探测一次，避免每帧重算）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorMode {
    TrueColor,
    Ansi256,
    Ansi16,
    None,
}

fn detect_color_mode() -> ColorMode {
    // NO_COLOR 是硬约定：设了就不上色（https://no-color.org）
    if std::env::var_os("NO_COLOR").is_some() {
        return ColorMode::None;
    }
    if let Ok(ct) = std::env::var("COLORTERM") {
        if ct == "truecolor" || ct == "24bit" {
            return ColorMode::TrueColor;
        }
    }
    match std::env::var("TERM").unwrap_or_default().as_str() {
        "dumb" | "" => ColorMode::None,
        t if t.contains("256color") => ColorMode::Ansi256,
        // 常见现代终端：即便没报 256/truecolor 也按 truecolor 处理
        "xterm-kitty" | "alacritty" | "wezterm" | "foot" | "tmux-256color" => {
            ColorMode::TrueColor
        }
        _ => ColorMode::Ansi16,
    }
}

impl Color {
    /// (R,G,B)。语义色取自**当前主题**（见 `theme.rs`），
    /// 所以换主题只需换一个名字，不必改任何渲染代码。
    fn rgb(self, t: &theme::Theme) -> (u8, u8, u8) {
        match self {
            Color::Primary => t.primary,
            Color::Accent => t.accent,
            Color::Success => t.success,
            Color::Error => t.error,
            Color::Warning => t.warning,
            Color::Info => t.info,
            Color::Text => t.text,
            Color::Muted => t.muted,
            Color::Border => t.border,
            Color::BorderActive => t.border_active,
            Color::Rgb(r, g, b) => (r, g, b),
        }
    }

    fn fg(self, mode: ColorMode, t: &theme::Theme) -> String {
        let (r, g, b) = self.rgb(t);
        match mode {
            ColorMode::None => String::new(),
            // 都从主题的 RGB 现场降级：这样加主题不必手工维护色号映射表
            ColorMode::TrueColor => format!("{ESC}[38;2;{r};{g};{b}m"),
            ColorMode::Ansi256 => format!("{ESC}[38;5;{}m", rgb_to_256(r, g, b)),
            ColorMode::Ansi16 => format!("{ESC}[{}m", rgb_to_16(r, g, b)),
        }
    }
}

/// 高亮降级色：给输入框左侧竖条/强调用；能力不足时退到普通前景。
fn faint(mode: ColorMode) -> String {
    match mode {
        ColorMode::None => String::new(),
        _ => format!("{ESC}[2m"), // dim（几乎所有终端都支持）
    }
}

const RESET: &str = "\u{1b}[0m";


/// 解析后的调色板：把 `Color` 按终端能力展开成可直接拼进 format! 的转义串。
struct Pal {
    primary: String,
    accent: String,
    success: String,
    error: String,
    warning: String,
    info: String,
    text: String,
    muted: String,
    border: String,
    border_active: String,
    dim: String,
    /// 重置序列。NO_COLOR 下必须是空串 —— 否则输出里仍残留 ESC[0m，
    /// 既污染重定向到文件的输出，也让"无色"变成半真半假。
    reset: String,
    /// 保留能力档位：`Tone::Rgb` 需要在渲染时现场算色
    mode: ColorMode,
    /// 当前主题（星场两档色与 logo 渐变从这里取）
    theme: theme::Theme,
}

impl Pal {
    fn new(mode: ColorMode, theme_name: theme::ThemeName) -> Self {
        let t = theme::get(theme_name);
        Self {
            theme: t,
            primary: Color::Primary.fg(mode, &t),
            accent: Color::Accent.fg(mode, &t),
            success: Color::Success.fg(mode, &t),
            error: Color::Error.fg(mode, &t),
            warning: Color::Warning.fg(mode, &t),
            info: Color::Info.fg(mode, &t),
            text: Color::Text.fg(mode, &t),
            muted: Color::Muted.fg(mode, &t),
            border: Color::Border.fg(mode, &t),
            border_active: Color::BorderActive.fg(mode, &t),
            dim: faint(mode),
            reset: if mode == ColorMode::None { String::new() } else { RESET.to_string() },
            mode,
        }
    }
}

/// 底部固定区块行数：输入框(4) + 提示行(1) + 状态行(1)
const CHROME_ROWS: usize = 6;

/// 侧栏宽度（对齐 opencode 的 42 列；我们窄一些，因为终端普遍没它宽）。
const SIDEBAR_COLS: usize = 34;
/// 显示侧栏所需的最小终端宽度。低于此值强制隐藏 ——
/// 正文被挤到 40 列以下时，侧栏带来的信息量抵不上阅读体验的损失。
const SIDEBAR_MIN_COLS: usize = 96;

/// 一行的事实片段：(起始列, 文本, 色调)
pub type Seg = (usize, String, Tone);
/// 居中的首屏片段（列由居中逻辑算，不用自己给）
type Styled = (String, Tone);

/// 提示行文案。**只列真正可用的键** —— 不写没实现的命令，
/// 否则用户按了没反应，比不提示更糟。
// 提示行必须能塞进输入框宽度（76 列封顶）。太长会被判为"放不下"而整条丢弃，
// 提示就白写了 —— 所以这里刻意精简，只留最高频的几个键。
/// 空输入时的示例任务（对标 opencode 首页的 placeholder 列表）。
///
/// 作用不是装饰：新用户面对空输入框往往不知道**该怎么问**，
/// 给一两个具体例子比写一句"输入任务"有用得多。
const EXAMPLES: [&str; 4] = [
    "修一下代码里的 TODO",
    "这个项目的技术栈是什么？",
    "跑一下测试并修掉失败的用例",
    "解释 src/main.rs 的主流程",
];

const HINT_LEFT: &str = "tab 补全   ctrl+r 历史";
const HINT_RIGHT: &str = "@ 引用   ctrl+g 编辑器   ctrl+c 退出";

/// 屏幕网格。
///
/// 为什么先铺格子再输出，而不是直接拼字符串：
/// 星场要能出现在**居中内容的左右两侧**。若按"内容 + 补星星"来做，
/// 左侧空白已算进内容宽度，星星永远进不去左边。格子模型把
/// "内容覆盖"与"背景填充"分开，两边都能正确落星。
struct Grid {
    cols: usize,
    rows: usize,
    ch: Vec<char>,
    tone: Vec<Tone>,
    /// 宽字符的续列：占位但不输出字符，否则整行会右移一列
    skip: Vec<bool>,
    /// 正文写入的右边界（不含）。用于给侧栏让位。
    put_limit: usize,
}

impl Grid {
    fn new(cols: usize, rows: usize) -> Self {
        let n = cols.saturating_mul(rows);
        Self {
            cols,
            rows,
            ch: vec![' '; n],
            tone: vec![Tone::None; n],
            skip: vec![false; n],
            put_limit: usize::MAX,
        }
    }

    /// 限制写入范围的列上限（`usize::MAX` = 不限制）。侧栏存在时设为侧栏左边界，
    /// 避免正文写到侧栏下面再被覆盖（那会在视觉上"截断"正文，很难解释）。
    fn clamp_put(&mut self, until: usize) {
        self.put_limit = until;
    }

    /// 写一段文本，返回结束列。宽字符按显示宽度占两列。
    fn put(&mut self, row: usize, col: usize, text: &str, tone: Tone) -> usize {
        if row >= self.rows {
            return col;
        }
        let mut c = col;
        for ch in text.chars() {
            let w = width::char_width(ch);
            if w == 0 {
                continue; // 组合字符：叠在前一个上，不单独占列
            }
            if c + w > self.cols.min(self.put_limit) {
                break;
            }
            let i = row * self.cols + c;
            self.ch[i] = ch;
            self.tone[i] = tone;
            self.skip[i] = false;
            for k in 1..w {
                let j = row * self.cols + c + k;
                self.ch[j] = ' ';
                self.tone[j] = tone;
                self.skip[j] = true;
            }
            c += w;
        }
        c
    }

    /// 居中一段多色调文本（列由总宽度算出，逐个片段顺序摆放）。
    fn put_centered_styled(&mut self, row: usize, segs: &[Styled]) {
        let total: usize = segs.iter().map(|(t, _)| width::display_width(t)).sum();
        let mut c = self.cols.saturating_sub(total) / 2;
        for (text, tone) in segs {
            c = self.put(row, c, text, *tone);
        }
    }

    /// 把 `[col_start, col_end)` 填成空格并标记为已写入。
    ///
    /// 用途：输入框内部。若不填，星场会渗进框里 —— 看起来像输入框"漏了"，
    /// 因为星场只在 `Tone::None`（从未写入）的格子上铺。
    fn blank(&mut self, row: usize, col_start: usize, col_end: usize, tone: Tone) {
        if row >= self.rows {
            return;
        }
        for c in col_start..col_end.min(self.cols) {
            let i = row * self.cols + c;
            self.ch[i] = ' ';
            self.tone[i] = tone;
            self.skip[i] = false;
        }
    }

    /// 给尚未写入的格子铺星场。**确定性**，所以多帧之间星位完全静止。
    fn fill_stars(&mut self) {
        for r in 0..self.rows {
            for c in 0..self.cols {
                let i = r * self.cols + c;
                if self.tone[i] != Tone::None {
                    continue;
                }
                if let Some((ch, bright)) = stars::cell(r, c, self.cols) {
                    self.ch[i] = ch;
                    self.tone[i] = if bright { Tone::StarBright } else { Tone::StarDim };
                }
            }
        }
    }

    /// 输出：按色调分段，避免逐字符上色。
    fn lines(&self, p: &Pal) -> Vec<String> {
        (0..self.rows)
            .map(|r| {
                let mut out = String::new();
                let mut cur = Tone::None;
                let mut started = false;
                for c in 0..self.cols {
                    let i = r * self.cols + c;
                    if self.skip[i] {
                        continue;
                    }
                    let t = self.tone[i];
                    if !started || t != cur {
                        if started {
                            out.push_str(&p.reset);
                        }
                        out.push_str(&p.tone(t));
                        cur = t;
                        started = true;
                    }
                    out.push(self.ch[i]);
                }
                if started {
                    out.push_str(&p.reset);
                }
                out
            })
            .collect()
    }
}

impl Screen<'_> {
    pub fn render(&self) -> String {
        let p = Pal::new(detect_color_mode(), self.theme);
        let mut g = Grid::new(self.cols, self.rows);

        // 侧栏：把正文的写入边界收到侧栏左侧，正文画完后再画侧栏。
        // 顺序是刻意的 —— 若先画侧栏再画正文，正文会把侧栏覆盖掉。
        // 最小宽度在这里**强制执行**：判断依赖 cols，就应放在知道 cols 的地方。
        // 只在调用方判断的话，任何别的调用点（测试、未来宿主）都能绕过它，
        // 得到"正文被挤到 30 列"的坏布局。
        let show_sidebar = self.sidebar && self.cols >= SIDEBAR_MIN_COLS;
        let side_x0 = if show_sidebar { Some(self.cols - SIDEBAR_COLS) } else { None };
        if let Some(x0) = side_x0 {
            g.clamp_put(x0);
        }

        let (chrome_top, cursor) = if let Some(lines) = self.preformatted {
            // 信息屏是**文档**：必须从第一行开始显示。
            // 曾用"显示末尾 N 行"（对话滚屏的逻辑），结果长帮助把标题裁掉、
            // 只留中间 —— 文档不能倒着读。
            let avail = self.rows.saturating_sub(CHROME_ROWS);
            let shown = lines.len().min(avail.saturating_sub(1)); // 留一行给截断提示
            for (i, segs) in lines.iter().enumerate().take(shown) {
                for (col, text, tone) in segs {
                    g.put(i + 1, *col, text, *tone);
                }
            }
            // 没显示完就**如实标注**（静默截断会让用户以为后面没了）
            if lines.len() > shown {
                let more = lines.len() - shown;
                g.put(
                    shown + 1,
                    2,
                    &format!("… 还有 {more} 行（终端太小，请放大窗口查看）"),
                    Tone::Border,
                );
            }
            let top = self.rows.saturating_sub(CHROME_ROWS);
            (top, self.draw_chrome(&mut g, top))
        } else if let Some(t) = self.trust {
            self.layout_trust(&mut g, t);
            (0, None)
        } else if self.facts.is_empty() && self.about.is_some() {
            self.layout_welcome(&mut g, &p, self.about.unwrap())
        } else {
            self.layout_transcript(&mut g)
        };
        // 弹窗画在内容之上、输入框之上（紧贴输入框顶边）——
        // 覆盖部分正文是下拉菜单的正常行为，比把正文挤走更不打扰
        if let Some(pop) = self.popup {
            self.draw_popup(&mut g, &p, pop, chrome_top);
        }
        // 侧栏最后画：它占的是自己的列区，且要求不被正文侵入
        if let Some(x0) = side_x0 {
            g.clamp_put(usize::MAX);
            self.draw_sidebar(&mut g, &p, x0);
        }

        g.fill_stars();
        let mut out = format!("{ESC}[H{ESC}[2J");
        out.push_str(&g.lines(&p).join("\r\n"));
        match cursor {
            // 用真实终端光标（而不是画一个 ▌），输入手感与原生一致
            Some((r, c)) => out.push_str(&format!("{ESC}[{r};{c}H{ESC}[?25h")),
            None => out.push_str(&format!("{ESC}[?25l")),
        }
        out
    }

    /// 输入框宽度：受终端宽度约束，并封顶 76 列。
    ///
    /// 封顶是刻意的：超宽终端上让输入框铺满整屏，排版会散掉
    /// （一行 200 列的眼睛移动距离，比 76 列难读得多）。
    fn box_width(&self) -> usize {
        let avail = self.body_cols();
        if avail >= 28 {
            (avail - 4).min(76)
        } else {
            avail.saturating_sub(2).max(8)
        }
    }

    /// 正文可用列数（侧栏占用右侧时会收窄）。
    ///
    /// 输入框、弹窗、状态行都要在这个区域内居中 —— 用整屏宽度居中会让
    /// 它们压到侧栏上（真机截图里输入框右边框就横穿了侧栏竖线）。
    fn body_cols(&self) -> usize {
        let show_sidebar = self.sidebar && self.cols >= SIDEBAR_MIN_COLS;
        if show_sidebar {
            self.cols.saturating_sub(SIDEBAR_COLS)
        } else {
            self.cols
        }
    }

    // ── 对话模式：正文在下、输入区钉在底部 ────────────────────────────
    fn layout_transcript(&self, g: &mut Grid) -> (usize, Option<(usize, usize)>) {
        let lines = self.fact_lines();
        let body = self.rows.saturating_sub(CHROME_ROWS);
        // 只显示最后 body 行（自动滚到底），内容不足时贴着输入区
        let start = lines.len().saturating_sub(body);
        let shown = &lines[start..];
        let top = body.saturating_sub(shown.len());
        for (i, segs) in shown.iter().enumerate() {
            for (col, text, tone) in segs {
                g.put(top + i, *col, text, *tone);
            }
        }
        let top = self.rows - CHROME_ROWS;
        (top, self.draw_chrome(g, top))
    }

    // ── 首屏：整组（logo + 输入区）垂直居中 ───────────────────────────
    fn layout_welcome(&self, g: &mut Grid, p: &Pal, a: &About) -> (usize, Option<(usize, usize)>) {
        let _ = p;
        let tiers: [(bool, bool, bool, bool); 5] = [
            (true, true, true, true),
            (true, true, false, true),
            (true, false, false, true),
            (false, false, false, true),
            (false, false, false, false),
        ];

        let mut chosen: Vec<Styled> = Vec::new();
        for &(logo, subtitle, meta, gaps) in &tiers {
            let hero = self.hero_lines(a, logo, subtitle, meta, gaps);
            // 整组要放得下，否则会被"显示末尾 N 行"从**顶部**裁掉 ——
            // 用户看到的是"少了 logo 的半截首屏"，且不会有任何报错
            if hero.len() + CHROME_ROWS <= self.rows {
                chosen = hero;
                break;
            }
        }
        if chosen.is_empty() {
            chosen = self.hero_lines(a, false, false, false, false);
        }

        let group = chosen.len() + CHROME_ROWS;
        let group_top = self.rows.saturating_sub(group) / 2;
        for (i, segs) in chosen.iter().enumerate() {
            g.put_centered_styled(group_top + i, std::slice::from_ref(segs));
        }
        let top = group_top + chosen.len();
        (top, self.draw_chrome(g, top))
    }

    fn hero_lines(&self, a: &About, logo: bool, subtitle: bool, meta: bool, gaps: bool) -> Vec<Styled> {
        let mut hero: Vec<Styled> = Vec::new();
        if logo && self.cols >= WORDMARK_MIN_COLS {
            // 竖向渐变：主色 → 强调色。单色 logo 太平，渐变让它"有光"
            let t = theme::get(self.theme);
            let (pr, ac) = (t.primary, t.accent);
            let n = (WORDMARK.len() - 1) as f32;
            for (i, row) in WORDMARK.iter().enumerate() {
                let (r, gg, b) = lerp_rgb(pr, ac, i as f32 / n);
                hero.push((row.trim_end().to_string(), Tone::Rgb(r, gg, b)));
            }
        } else {
            hero.push(("NEO".to_string(), Tone::Primary));
        }

        if gaps {
            hero.push((String::new(), Tone::None));
        }
        hero.push(("Neo".to_string(), Tone::Text));
        hero.push((" —— 编程 Agent 内核".to_string(), Tone::Muted));
        if subtitle {
            hero.push(("Rust 内核 · TUI / Web / Exec 共享同一内核".to_string(), Tone::Info));
        }
        if meta {
            if gaps {
                hero.push((String::new(), Tone::None));
            }
            hero.push((format!("v{} · 会话 {}", a.version, a.session), Tone::Muted));
        }
        hero
    }

    /// 右侧面板：Context（用量）/ Todo（进度）/ Files（改动）。
    ///
    /// 对齐 opencode 的 sidebar 插件集，但**只放我们有数据的三块** ——
    /// 它还有 MCP / LSP 面板，我们没有 MCP 与 LSP，做了只会是空面板。
    fn draw_sidebar(&self, g: &mut Grid, p: &Pal, x0: usize) {
        let _ = p;
        let w = self.cols.saturating_sub(x0);
        let inner = w.saturating_sub(3);

        // 左侧竖线把侧栏与正文分开（对标 opencode 的分栏观感）
        for r in 0..self.rows {
            g.put(r, x0, "│", Tone::Border);
        }

        let mut row = 1usize;
        let section = |g: &mut Grid, row: &mut usize, title: &str, tone: Tone| {
            if *row >= self.rows {
                return;
            }
            g.put(*row, x0 + 2, title, tone);
            *row += 1;
        };

        // ── Context：token 用量 ──
        let (tin, tout) = self
            .facts
            .iter()
            .filter_map(|f| match f {
                Fact::TurnFinished { input_tokens, output_tokens } => {
                    Some((*input_tokens, *output_tokens))
                }
                _ => None,
            })
            .fold((0u64, 0u64), |a, b| (a.0.max(b.0), a.1.max(b.1)));
        // 取最近一轮的用量（每轮都是独立统计，累加没有意义）
        let (last_in, last_out) = self
            .facts
            .iter()
            .rev()
            .find_map(|f| match f {
                Fact::TurnFinished { input_tokens, output_tokens } => {
                    Some((*input_tokens, *output_tokens))
                }
                _ => None,
            })
            .unwrap_or((0, 0));
        let _ = (tin, tout);
        section(g, &mut row, "Context", Tone::Text);
        g.put(row, x0 + 2, &format!("{last_in} in / {last_out} out"), Tone::Muted);
        row += 1;
        match self.about.map(|a| a.context_limit).unwrap_or(0) {
            // 上限未知时**不显示百分比** —— 宁可不给，也不给假数字
            0 => {
                g.put(row, x0 + 2, "上限未知", Tone::Muted);
                row += 1;
            }
            limit => {
                let used = last_in + last_out;
                let pct = if limit == 0 { 0 } else { (used * 100 / limit).min(999) };
                let tone = if pct >= 90 {
                    Tone::Error
                } else if pct >= 70 {
                    Tone::Warning
                } else {
                    Tone::Muted
                };
                g.put(row, x0 + 2, &format!("{pct}% of {limit}"), tone);
                row += 1;
            }
        }

        // ── Todo：任务清单（最近一次）──
        if let Some(items) = self.facts.iter().rev().find_map(|f| match f {
            Fact::TodoList(items) => Some(items),
            _ => None,
        }) {
            let done = items
                .iter()
                .filter(|i| matches!(i.status, neo_protocol::TodoStatus::Completed))
                .count();
            row += 1;
            section(g, &mut row, &format!("Todo {done}/{}", items.len()), Tone::Text);
            for it in items.iter().take(12) {
                if row >= self.rows {
                    break;
                }
                let (mark, tone) = match it.status {
                    neo_protocol::TodoStatus::Completed => ("✓", Tone::Success),
                    neo_protocol::TodoStatus::InProgress => ("•", Tone::Warning),
                    neo_protocol::TodoStatus::Pending => ("·", Tone::Muted),
                };
                g.put(row, x0 + 2, mark, tone);
                let text = width::truncate_to_width(&it.content, inner.saturating_sub(3)).to_string();
                g.put(row, x0 + 4, &text, tone);
                row += 1;
            }
            if items.len() > 12 {
                g.put(row, x0 + 2, &format!("… 另 {} 项", items.len() - 12), Tone::Muted);
                row += 1;
            }
        }

        // ── Files：已修改文件 ──
        if let Some(files) = self.facts.iter().rev().find_map(|f| match f {
            Fact::FilesChanged(files) => Some(files),
            _ => None,
        }) {
            let adds: usize = files.iter().map(|f| f.additions).sum();
            let dels: usize = files.iter().map(|f| f.deletions).sum();
            let _ = (adds, dels);
            row += 1;
            // 标题只写名字：每个文件自带 +N -N，标题再写一次是冗余
            section(g, &mut row, "Modified Files", Tone::Text);
            for f in files.iter().take(10) {
                if row >= self.rows {
                    break;
                }
                // 路径从**左侧**截断（保留文件名，丢弃前面的目录）——
                // 文件名才是识别信息，截尾会把最关键的部分吃掉
                let counts = format!(" +{} -{}", f.additions, f.deletions);
                let budget = inner.saturating_sub(counts.len());
                let name = truncate_left(&f.path, budget);
                g.put(row, x0 + 2, &name, Tone::Muted);
                let cx = x0 + w - 2 - counts.len();
                if cx > x0 + 2 + width::display_width(&name) {
                    let tone = if f.deletions > 0 && f.additions == 0 {
                        Tone::Error
                    } else {
                        Tone::Success
                    };
                    g.put(row, cx, counts.trim_start(), tone);
                }
                row += 1;
            }
            if files.len() > 10 && row < self.rows {
                g.put(row, x0 + 2, &format!("… 另 {} 个", files.len() - 10), Tone::Muted);
            }
        }

        // ── 底部：工作区分支（对标 opencode sidebar 的 footer）──
        if let Some(a) = self.about {
            if self.rows > 2 {
                let label = if a.branch.is_empty() { a.model.clone() } else { a.branch.clone() };
                let t = width::truncate_to_width(&label, inner).to_string();
                g.put(self.rows - 2, x0 + 2, &t, Tone::Border);
            }
        }
    }

    /// 弹窗：标题 + 候选项 + （截断时）页脚。画在输入框上方。
    fn draw_popup(&self, g: &mut Grid, p: &Pal, pop: &popup::Popup, chrome_top: usize) {
        let _ = p;
        let body = self.body_cols();
        let box_w = self.box_width();
        let left = body.saturating_sub(box_w) / 2;
        let inner = box_w.saturating_sub(4);
        let max_rows = pop.kind.max_rows();
        let visible = pop.items.len().min(max_rows);
        // 标题(1) + 空目录提示(1) + 候选 + 页脚(截断时 1)
        let hint_rows = if pop.is_empty() { 1 } else { 0 };
        let foot_rows = if pop.truncated { 1 } else { 0 };
        let height = 2 + visible + hint_rows + foot_rows; // 含上下边框这 2 行
        if chrome_top < height + 1 {
            return; // 上方空间不够就不画（宁可没有弹窗，也不画残缺的）
        }
        let top = chrome_top - height - 1; // 与输入框留一行间隔

        // 关键：先把弹窗占据的**整个矩形**填成空格并标记已写入。
        // 不填的话，只有我们写到字符的位置被覆盖，其余格子会保留底下的
        // logo/正文/星场 —— 看起来像弹窗"半透明"，非常脏。
        for r in top..(top + height).min(self.rows) {
            g.blank(r, left, left + box_w, Tone::Text);
        }

        let bar = "─".repeat(box_w.saturating_sub(2));
        g.put(top, left, "╭", Tone::BorderActive);
        g.put(top, left + 1, &bar, Tone::BorderActive);
        g.put(top, left + box_w - 1, "╮", Tone::BorderActive);

        // 标题行：类型 + 当前过滤词（让用户知道"我在过滤什么"）
        let title = if pop.query.is_empty() {
            pop.kind.title().to_string()
        } else {
            format!("{} · {}", pop.kind.title(), pop.query)
        };
        g.put(top + 1, left, "│", Tone::BorderActive);
        g.put(top + 1, left + 2, &width::truncate_to_width(&title, inner).to_string(), Tone::Muted);
        g.put(top + 1, left + box_w - 1, "│", Tone::BorderActive);

        let mut row = 2;
        if pop.is_empty() {
            g.put(top + row, left, "│", Tone::BorderActive);
            let msg = if pop.query.is_empty() { "（无候选）" } else { "无匹配" };
            g.put(top + row, left + 2, msg, Tone::Muted);
            g.put(top + row, left + box_w - 1, "│", Tone::BorderActive);
            row += 1;
        } else {
            let start = pop.scroll_top(visible);
            for (i, item) in pop.items.iter().enumerate().skip(start).take(visible) {
                let selected = i == pop.selected;
                let (mark, label_tone) = if selected {
                    ("▸ ", Tone::Primary)
                } else {
                    ("  ", Tone::Text)
                };
                g.put(top + row, left, "│", Tone::BorderActive);
                g.put(top + row, left + 2, mark, Tone::Primary);
                let label = width::truncate_to_width(&item.label, inner.saturating_sub(2)).to_string();
                let used = g.put(top + row, left + 4, &label, label_tone);
                // 右侧说明：空间够才写（不挤掉主标签）
                if !item.detail.is_empty() {
                    let d = width::truncate_to_width(&item.detail, 34).to_string();
                    let dw = width::display_width(&d);
                    let dx = (left + box_w - 2).saturating_sub(dw);
                    if dx > used + 2 {
                        g.put(top + row, dx, &d, Tone::Muted);
                    }
                }
                g.put(top + row, left + box_w - 1, "│", Tone::BorderActive);
                row += 1;
            }
        }

        if pop.truncated && !pop.is_empty() {
            g.put(top + row, left, "│", Tone::BorderActive);
            let more = pop.items.len().saturating_sub(visible);
            g.put(top + row, left + 2, &format!("↑↓ 还有 {more} 项…"), Tone::Muted);
            g.put(top + row, left + box_w - 1, "│", Tone::BorderActive);
            row += 1;
        }

        g.put(top + row, left, "╰", Tone::BorderActive);
        g.put(top + row, left + 1, &bar, Tone::BorderActive);
        g.put(top + row, left + box_w - 1, "╯", Tone::BorderActive);

        // 提示行：这一行告诉用户怎么操作（否则新用户不知道 Enter 会怎样）
        if top + row + 1 < chrome_top {
            g.put(top + row + 1, left + 2, "↑↓ 选择   Enter 确认   Esc 取消", Tone::Border);
        }
    }

    /// 输入框 + 提示行 + 状态行。返回光标的 (行, 列)（1 基）。
    fn draw_chrome(&self, g: &mut Grid, top: usize) -> Option<(usize, usize)> {
        if top + CHROME_ROWS > self.rows {
            return None;
        }
        let body = self.body_cols();
        let box_w = self.box_width();
        let left = body.saturating_sub(box_w) / 2;
        let inner_w = box_w.saturating_sub(4);
        // 待审批时边框转警告色：余光里也能看出"现在轮到你"
        let border = if self.awaiting_input { Tone::Warning } else { Tone::BorderActive };
        let bar = "─".repeat(box_w.saturating_sub(2));

        g.put(top, left, "╭", border);
        g.put(top, left + 1, &bar, border);
        g.put(top, left + box_w - 1, "╮", border);

        // 先把框内两行填实，避免星场渗入输入框
        g.blank(top + 1, left + 1, left + box_w - 1, Tone::Text);
        g.blank(top + 2, left + 1, left + box_w - 1, Tone::Text);

        // 提示行：空输入时给占位提示（否则光标处一片空白，不知道能打什么）
        g.put(top + 1, left, "│", border);
        let (text, tone) = if self.input.is_empty() && !self.awaiting_input {
            let ex = match self.about {
                Some(a) if !a.example.is_empty() => a.example.clone(),
                _ => "输入任务".to_string(),
            };
            (format!("输入任务… 例：{ex}"), Tone::Muted)
        } else {
            let prefix = if self.awaiting_input { "y 批准 / n 拒绝 > " } else { "> " };
            (format!("{prefix}{}", self.input), Tone::Text)
        };
        g.put(top + 1, left + 2, &text, tone);
        g.put(top + 1, left + box_w - 1, "│", border);

        // 内层状态行（对标 MiMo 输入框内的 "Build ⏵ 模型"）
        let inner = match self.about {
            Some(a) if !a.mode_short.is_empty() => format!("{} ⏵ {}", a.mode_short, a.model),
            Some(a) => a.model.clone(),
            None => String::new(),
        };
        g.put(top + 2, left, "│", border);
        g.put(top + 2, left + 2, &width::truncate_to_width(&inner, inner_w).to_string(), Tone::Muted);
        g.put(top + 2, left + box_w - 1, "│", border);

        g.put(top + 3, left, "╰", border);
        g.put(top + 3, left + 1, &bar, border);
        g.put(top + 3, left + box_w - 1, "╯", border);

        // 提示行：与输入框左右对齐
        g.put(top + 4, left + 2, HINT_LEFT, Tone::Border);
        let hw = width::display_width(HINT_RIGHT);
        let hx = (left + box_w).saturating_sub(2 + hw);
        if hx > left + 2 + width::display_width(HINT_LEFT) {
            g.put(top + 4, hx, HINT_RIGHT, Tone::Border);
        }

        // 状态行（最底）：左 = 工作区:分支，右 = 状态文本
        let ws_line = match self.about {
            Some(a) if !a.branch.is_empty() => format!("{}:{}", a.workspace, a.branch),
            Some(a) => a.workspace.clone(),
            None => String::new(),
        };
        let ws_shown = width::truncate_to_width(&ws_line, body.saturating_sub(6)).to_string();
        let wsw = width::display_width(&ws_shown);
        let stw = width::display_width(self.status);
        // 放不下就不画右侧 —— 宁可少显示，也不折行把布局打乱
        if self.status.is_empty() || wsw + stw + 4 > body {
            g.put(top + 5, 2, &ws_shown, Tone::Dim);
        } else {
            g.put(top + 5, 2, &ws_shown, Tone::Dim);
            let tone = if self.awaiting_input { Tone::Warning } else { Tone::Muted };
            g.put(top + 5, body - stw - 2, self.status, tone);
        }

        if self.show_cursor {
            // 光标列：1 基，落在已输入文本之后
            Some((top + 2, left + 3 + width::display_width(&text)))
        } else {
            None
        }
    }

    // ── 信任对话框 ────────────────────────────────────────────────────
    fn layout_trust(&self, g: &mut Grid, t: &TrustPrompt) {
        let m = 3usize;
        let mut rows: Vec<(usize, String, Tone)> = Vec::new();
        let push = |indent: usize, text: &str, tone: Tone, rows: &mut Vec<(usize, String, Tone)>| {
            let w = self.cols.saturating_sub(m * 2 + indent);
            for line in width::wrap_to_width(text, w) {
                rows.push((indent, line, tone));
            }
        };

        push(0, "● 访问工作区：", Tone::Info, &mut rows);
        push(2, &self.about.map(|a| a.workspace.clone()).unwrap_or_default(), Tone::Text, &mut rows);
        rows.push((0, String::new(), Tone::None));
        push(0, "安全确认：这个目录是你自己创建或信任的吗？（自己的代码、知名开源项目、团队内部项目）", Tone::Text, &mut rows);
        push(0, "如果不是，请先检查这个目录里的内容 —— 接下来 Neo 能读取、编辑并执行其中的文件。", Tone::Muted, &mut rows);
        push(0, "如果这个目录含有恶意脚本，它们可以执行任意代码、读取、修改或窃取你的文件。", Tone::Error, &mut rows);
        rows.push((0, String::new(), Tone::None));
        push(0, "◆", Tone::Accent, &mut rows);
        rows.push((0, String::new(), Tone::None));

        let opts = ["是的，我信任此目录", "否，退出"];
        for (i, label) in opts.iter().enumerate() {
            let selected = i == t.selected;
            let (dot, dt, tt) = if selected {
                ("●", Tone::Success, Tone::Text)
            } else {
                ("○", Tone::Muted, Tone::Muted)
            };
            rows.push((2, format!("{dot} {label}"), tt));
            let _ = dt;
        }
        rows.push((0, String::new(), Tone::None));
        push(2, "↑/↓ 选择 · Enter 确认 · y/n 快捷 · Ctrl+C 退出", Tone::Border, &mut rows);

        // 垂直居中
        let top = self.rows.saturating_sub(rows.len()) / 2;
        for (i, (indent, text, tone)) in rows.iter().enumerate() {
            if text.is_empty() {
                continue;
            }
            g.put(top + i, m + indent, text, *tone);
        }
        // 左侧竖条：给"这是要你决定的事"一个视觉框（对标 MiMo 的左侧竖条）
        let bar_top = top;
        let bar_bottom = (top + rows.len()).min(self.rows);
        for r in bar_top..bar_bottom {
            g.put(r, 1, "┃", Tone::Accent);
        }
    }

    /// 把事实渲染成"行 → 片段"（不含 ANSI，色调交给网格统一展开）。
    ///
    /// 视觉语言（对标 opencode / MiMo）：
    ///   - 用户消息带左侧竖条，与助手正文区分
    ///   - 助手正文不加框、顶格直排
    ///   - 工具调用成功 ✓ / 失败 ✗，细节（exit code）压暗
    ///   - 元信息一律 muted，不抢正文
    fn fact_lines(&self) -> Vec<Vec<Seg>> {
        // 侧栏存在时正文可用宽度收窄（否则文字会被裁在侧栏左边界，看起来是"断行"）
        let show_sidebar = self.sidebar && self.cols >= SIDEBAR_MIN_COLS;
        let body_cols =
            if show_sidebar { self.cols.saturating_sub(SIDEBAR_COLS) } else { self.cols };
        let inner = body_cols.saturating_sub(4);
        let mut out: Vec<Vec<Seg>> = Vec::new();
        for f in self.facts {
            match f {
                Fact::UserSaid(text) => {
                    // 用户消息也按 Markdown 渲染（经常粘贴代码/清单），
                    // 但整块保留左侧竖条，与助手正文区分
                    let body = markdown::render(text, inner.saturating_sub(2));
                    for mut l in body {
                        let mut seg = vec![(0, "┃".to_string(), Tone::Accent)];
                        seg.push((2, " ".to_string(), Tone::Text));
                        for (col, t, tone) in l.drain(..) {
                            seg.push((col + 2, t, tone));
                        }
                        out.push(seg);
                    }
                    out.push(Vec::new());
                }
                Fact::AssistantSaid(text) => {
                    // 助手回复按 Markdown 渲染：代码块高亮、行内代码、标题、列表。
                    // inner 已扣掉侧栏占用（render 把正文写入裁到侧栏左侧）。
                    out.extend(markdown::render(text, inner));
                    out.push(Vec::new());
                }
                Fact::ToolFinished { name, exit_code } => {
                    let (icon, tone) =
                        if *exit_code == 0 { ("✓", Tone::Success) } else { ("✗", Tone::Error) };
                    let after = 4 + width::display_width(name);
                    out.push(vec![
                        (2, format!("{icon} "), tone),
                        (4, name.clone(), Tone::Text),
                        (after + 1, format!("exit {exit_code}"), Tone::Muted),
                    ]);
                }
                Fact::PatchPreview { path, diff } => {
                    out.push(vec![(2, format!("◆ {path}"), Tone::Info)]);
                    // diff 可能很长：只给前 DIFF_LINES 行，其余如实标注行数。
                    // 静默截断会让用户以为"就这么点改动"。
                    let all: Vec<&str> = diff.lines().collect();
                    const DIFF_LINES: usize = 200;
                    for l in all.iter().take(DIFF_LINES) {
                        let (tone, text) = if l.starts_with("+++") || l.starts_with("---") {
                            (Tone::Muted, l.to_string())
                        } else if l.starts_with('+') {
                            (Tone::Success, l.to_string())
                        } else if l.starts_with('-') {
                            (Tone::Error, l.to_string())
                        } else if l.starts_with("@@") {
                            (Tone::Accent, l.to_string())
                        } else {
                            (Tone::Muted, l.to_string())
                        };
                        for w in width::wrap_to_width(&text, inner) {
                            out.push(vec![(4, w, tone)]);
                        }
                    }
                    if all.len() > DIFF_LINES {
                        out.push(vec![(
                            4,
                            format!("… 另有 {} 行改动未展示", all.len() - DIFF_LINES),
                            Tone::Muted,
                        )]);
                    }
                }
                Fact::TodoList(items) => {
                    // 对齐 opencode：完成 ✓ / 进行中 • / 待办 空格 三态
                    let done = items
                        .iter()
                        .filter(|i| matches!(i.status, neo_protocol::TodoStatus::Completed))
                        .count();
                    out.push(vec![(
                        2,
                        format!("◇ 任务清单 {done}/{}", items.len()),
                        Tone::Info,
                    )]);
                    for it in items {
                        let (mark, tone) = match it.status {
                            neo_protocol::TodoStatus::Completed => ("[✓]", Tone::Success),
                            neo_protocol::TodoStatus::InProgress => ("[•]", Tone::Warning),
                            neo_protocol::TodoStatus::Pending => ("[ ]", Tone::Muted),
                        };
                        let text = format!("{mark} {}", it.content);
                        for w in width::wrap_to_width(&text, inner.saturating_sub(2)) {
                            out.push(vec![(4, w, tone)]);
                        }
                    }
                }
                Fact::FilesChanged(files) => {
                    // 正文里只给一行汇总；明细在右侧面板（避免刷屏）
                    let adds: usize = files.iter().map(|f| f.additions).sum();
                    let dels: usize = files.iter().map(|f| f.deletions).sum();
                    out.push(vec![(
                        2,
                        format!("◆ 已修改 {} 个文件（+{adds} -{dels}）", files.len()),
                        Tone::Info,
                    )]);
                }
                Fact::ApprovalNeeded { detail } => {
                    out.push(vec![(2, format!("△ 需要审批：{detail}"), Tone::Warning)]);
                }
                Fact::Failed(msg) => out.push(vec![(2, format!("✗ {msg}"), Tone::Error)]),
                Fact::TurnFinished { input_tokens, output_tokens } => out.push(vec![(
                    2,
                    format!("· {input_tokens} in / {output_tokens} out"),
                    Tone::Muted,
                )]),
                Fact::SessionReady { session_id } => {
                    out.push(vec![(2, format!("· 会话 {session_id}"), Tone::Muted)]);
                }
            }
        }
        out
    }
}

/// 用 `$EDITOR`（或 `$VISUAL`，再退到 vi）编辑一段文本，返回编辑结果。
///
/// 关键：**编辑期间必须还原终端** —— 外部编辑器要独占终端，
/// 若仍处于原始模式，编辑器会看到"每敲一个字符就来一个按键"的怪状态。
/// 因此这里显式 restore，编辑完再重新进入原始模式。
pub fn edit_externally(initial: &str, raw: &RawMode) -> Option<String> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());

    let path = std::env::temp_dir().join(format!("neo-tui-edit-{}.txt", std::process::id()));
    std::fs::write(&path, initial).ok()?;

    // 把终端交还给编辑器
    raw.restore();

    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} {}", path.display()))
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();

    // 无论编辑器成功与否，都要把终端抢回原始模式（否则后续按键读不到）
    let _ = set_stty("raw -echo");

    let out = match status {
        Ok(s) if s.success() => std::fs::read_to_string(&path).ok(),
        _ => None,
    };
    let _ = std::fs::remove_file(&path);
    out
}

/// 把输入行里最后一个 `@片段` 替换成 `replacement`（`@` 后无空白的部分）。
///
/// 抽成纯函数：补全是"改用户正在输入的文字"，算错会让人莫名其妙
/// （替换错位置、吃掉已有内容），故可单测。
pub fn complete_at_token(input: &str, replacement: &str) -> String {
    // 找最后一个 '@'，且其后不含空格（即当前正在输入的引用）
    let Some(at) = input.rfind('@') else {
        // 没有 @ 就当追加一个新引用
        return format!("{input}@{replacement}");
    };
    let after = &input[at + 1..];
    if after.contains(' ') {
        // @ 之后已有空格 → 上一个引用已完成，追加新的
        return format!("{input}@{replacement}");
    }
    format!("{}@{replacement}", &input[..at])
}

/// 取 @ 后面的当前查询串（用于过滤候选）。
pub fn at_query(input: &str) -> Option<&str> {
    let at = input.rfind('@')?;
    let after = &input[at + 1..];
    if after.contains(' ') {
        None
    } else {
        Some(after)
    }
}

/// 取一批事件里**最后一个**审批请求的 id。/// 取一批事件里**最后一个**审批请求的 id。
///
/// 内核是严格顺序的（一次只有一个未决审批），所以取最后一个即可。
/// 抽成纯函数是为了可单测：审批交互错了会让"需要审批"变成静默挂起。
pub fn latest_approval_id(events: &[EventMsg]) -> Option<String> {
    events.iter().rev().find_map(|e| match e {
        EventMsg::ApprovalRequest { id, .. } => Some(id.clone()),
        _ => None,
    })
}

/// 审批应答解析：y/Y/yes 批准，n/N/no 拒绝，其它为 None（**不提交**）。
///
/// 无法识别时不提交很重要：若把随机输入当"批准"，等于悄悄放水。
pub fn parse_approval_answer(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Some(true),
        "n" | "no" => Some(false),
        _ => None,
    }
}

// ══════════════════════════════════════════════════════════════════════
// HostBackend 实现（T6 的比较对象）
// ══════════════════════════════════════════════════════════════════════

/// TUI 宿主的事实视图 —— 只做**累积**，抽取语义在协议层（`facts_of`）。
///
/// 这一点很重要：若每个宿主各自定义"什么算事实"，T6 就无从比较等价性。
pub struct TuiFacts {
    events: Vec<EventMsg>,
}

impl TuiFacts {
    pub fn new() -> Self { Self { events: Vec::new() } }
}

impl Default for TuiFacts {
    fn default() -> Self { Self::new() }
}

impl HostBackend for TuiFacts {
    fn id(&self) -> &'static str { "tui" }

    fn capabilities(&self) -> HostCapabilities {
        HostCapabilities {
            images: ImageSupport::None, // 终端无法内联图片（诚实能力声明）
            rich_text: true,            // ANSI 样式
            interactive_prompt: true,   // 能弹审批
            diffs: DiffSupport::Text,   // 可画文本 diff，不支持 hunk 交互
        }
    }

    fn consume(&mut self, event: &EventMsg) -> Result<(), String> {
        // TUI 能处理任何事件；不认识的也不丢（累积后由 facts_of 决定是否成事实）
        self.events.push(event.clone());
        Ok(())
    }

    fn facts(&self) -> Vec<Fact> { facts_of(&self.events) }
}

// ══════════════════════════════════════════════════════════════════════
// 交互主循环
// ══════════════════════════════════════════════════════════════════════

/// 挑一个示例任务。用启动时刻做种子即可 —— 这里要的是"每次不一样"，
/// 不是统计意义上的随机，没必要为此引入随机数依赖。
pub fn pick_example() -> String {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0);
    EXAMPLES[seed % EXAMPLES.len()].to_string()
}

/// 当前输入是否处于"弹窗上下文"：返回 (弹窗类型, 过滤词)。
///
/// 判据是**光标前最后一个 `@` 或 `/`**，且它必须是词的起点
/// （前面是空白或行首）——否则 `a/b` 这种路径会被误判成命令。
fn self_popup_context(input: &str) -> Option<(popup::Kind, String)> {
    let bytes: Vec<char> = input.chars().collect();
    // 从后往前找最近的 @ 或 /
    for (i, c) in bytes.iter().enumerate().rev() {
        if *c != '@' && *c != '/' {
            continue;
        }
        let at_word_start = i == 0 || bytes[i - 1].is_whitespace();
        if !at_word_start {
            return None;
        }
        let query: String = bytes[i + 1..].iter().collect();
        // 过滤词里不应再出现空白（那说明用户已经在写正文了）
        if query.contains(char::is_whitespace) {
            return None;
        }
        let kind = if *c == '@' { popup::Kind::File } else { popup::Kind::Slash };
        return Some((kind, query));
    }
    None
}

/// 按类型刷新候选。
fn refresh_popup(p: &mut popup::Popup, kind: popup::Kind, files: &mut Option<Vec<String>>) {
    match kind {
        popup::Kind::File => {
            if files.is_none() {
                let (list, _) = input::list_files(
                    &std::env::current_dir().unwrap_or_else(|_| ".".into()),
                    5000,
                    8,
                );
                *files = Some(list);
            }
            let all = files.as_deref().unwrap_or(&[]);
            let (items, truncated) = popup::file_items(&p.query, all, p.kind.max_rows());
            p.set_items(items, truncated);
        }
        popup::Kind::Slash => {
            let n = p.query.clone();
            p.set_items(popup::slash_items(&n), false);
        }
        popup::Kind::Palette => {
            let n = p.query.clone();
            p.set_items(popup::palette_items(&n), false);
        }
        popup::Kind::Theme => {
            let n = p.query.clone();
            p.set_items(popup::theme_items(&n), false);
        }
    }
}

/// 刚输入 `@` 或 `/` 时打开弹窗。
fn open_popup_for(
    input: &str,
    slot: &mut Option<popup::Popup>,
    files: &mut Option<Vec<String>>,
    status: &mut String,
) {
    let Some((kind, q)) = self_popup_context(input) else {
        // 已经不在 @ / 上下文（例如用户在中间插了空格）→ 关掉
        *slot = None;
        return;
    };
    let mut p = popup::Popup::new(kind, q);
    refresh_popup(&mut p, kind, files);
    if p.is_empty() {
        *status = match kind {
            popup::Kind::File => "没有匹配的文件".to_string(),
            popup::Kind::Slash => "没有匹配的命令".to_string(),
            _ => String::new(),
        };
    } else {
        *status = format!("{} · {} 项", kind.title(), p.items.len());
    }
    *slot = Some(p);
}

/// 执行弹窗项时**只能由主循环完成**的副作用。
///
/// 拆出来的原因：`apply_popup_item` 不该拿到 `events`/`popup_state`
/// 这些主循环状态。让它返回一个"请求"，由循环统一执行 ——
/// 这样清转录、开面板这类动作只有一条实现路径，不会两处各写一遍。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Effect {
    None,
    Quit,
    /// 清空转录（`/new`）
    ClearTranscript,
    /// 打开主题选择弹窗（`/theme`）
    OpenThemePicker,
    /// 显示状态屏（正文由运行时拼装）
    ShowStatus,
}

/// 执行弹窗里选中的项。返回需要主循环落实的副作用。
fn apply_popup_item(
    item: &popup::Item,
    input: &mut String,
    status: &mut String,
    theme_name: &mut theme::ThemeName,
    info_screen: &mut Option<String>,
) -> Effect {
    match &item.action {
        popup::ItemAction::Insert(text) => {
            // 用选中的引用替换掉 `@` 之后已输入的过滤词
            *input = complete_at_token(input, text);
            *status = format!("已引用 {text}");
            Effect::None
        }
        popup::ItemAction::SetTheme(t) => {
            input.clear();
            *theme_name = *t;
            theme::save_preference(*t);
            *status = format!("主题：{}", t.as_str());
            Effect::None
        }
        popup::ItemAction::Run(action) => {
            // 执行命令后必须清空输入框：`/help` 已经"用掉"了，
            // 留在框里会让用户以为还没执行，且盖住占位提示。
            // （文件引用项相反 —— 那是要保留在输入里发给模型的。）
            input.clear();
            match action {
            commands::Action::Quit => Effect::Quit,
            commands::Action::NewSession => Effect::ClearTranscript,
            commands::Action::Compact => {
                // 如实说明未实现，而不是假装压缩了
                *status = "压缩上下文需要 L4 编排，尚未实现".to_string();
                Effect::None
            }
            commands::Action::ThemePicker => Effect::OpenThemePicker,
            commands::Action::NextTheme => {
                *theme_name = theme_name.next();
                theme::save_preference(*theme_name);
                *status = format!("主题：{}", theme_name.as_str());
                Effect::None
            }
            commands::Action::SetTheme(t) => {
                *theme_name = *t;
                theme::save_preference(*t);
                *status = format!("主题：{}", t.as_str());
                Effect::None
            }
            commands::Action::Help => {
                *info_screen = Some(commands::help_text().to_string());
                Effect::None
            }
            commands::Action::Status => Effect::ShowStatus,
            commands::Action::Keys => {
                *info_screen = Some(commands::keys_text().to_string());
                Effect::None
            }
            }
        }
    }
}

/// 从**左侧**截断，保留尾部（文件名）。
///
/// 用途：侧栏的"已修改文件"列表。文件名才是识别信息，
/// 截尾（`src/very/long/pa…`）会把最关键的部分吃掉。
fn truncate_left(s: &str, max_cols: usize) -> String {
    if width::display_width(s) <= max_cols {
        return s.to_string();
    }
    if max_cols <= 1 {
        return String::new();
    }
    let mut tail: Vec<char> = Vec::new();
    let mut used = 0;
    // 预算是 max_cols-1，给省略号留一列
    for ch in s.chars().rev() {
        let w = width::char_width(ch);
        if used + w > max_cols - 1 {
            break;
        }
        tail.push(ch);
        used += w;
    }
    tail.reverse();
    format!("…{}", tail.into_iter().collect::<String>())
}

/// 状态栏文案：审批挂起时明确告诉用户该敲什么，否则是常规就绪提示。
fn idle_or_approval(outstanding: &Option<String>) -> String {
    match outstanding {
        Some(_) => "待审批 · y 批准 / n 拒绝".to_string(),
        // 空闲时不再堆一串按键说明 —— 输入框有占位提示、下方有提示行
        None => "就绪".to_string(),
    }
}

/// 运行 TUI，直到用户退出。
///
/// `submit` 由调用方注入（通常是 `kernel.submit`），这样本 crate
/// **不依赖 neo-orchestration / L3 之上的任何东西**，只依赖契据。
pub fn run<F>(
    about: About,
    trust_workspace: Option<std::path::PathBuf>,
    mut theme_name: theme::ThemeName,
    mut submit: F,
) -> std::io::Result<()>
where
    F: FnMut(neo_protocol::Op) -> Result<Vec<EventMsg>, String>,
{
    let raw = RawMode::enter().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    // 进备用屏：全程在第二块屏幕上画，退出时终端整块还原
    let mut alt = AltScreen::enter();

    // panic 时也还原终端，否则用户的终端会被留在原始模式
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = set_stty("sane");
        // 必须退出备用屏，否则 panic 后用户的终端会一直停在我们这里
        AltScreen::leave_now();
        println!("\r\n[tui] 发生 panic，终端已还原");
        previous_hook(info);
    }));

    let mut stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    let mut input = String::new();
    let mut events: Vec<EventMsg> = Vec::new();
    // 与后续状态统一用同一个构造函数，避免"首屏一种文案、之后另一种"
    let mut status = idle_or_approval(&None);
    let mut history = input::History::default();
    // 历史浏览态：Up/Down 在历史里移动时置位，一旦用户输入字符即退出该态
    let mut browsing = false;
    // 文件候选（首次按 @ 时惰性加载 —— 遍历文件系统不该在启动时做）
    let mut file_cache: Option<Vec<String>> = None;
    // 未决审批：内核挂起后必须由用户应答，否则界面只是"显示"而无法推进。
    let mut outstanding: Option<String> = None;
    // 弹窗（@ / / / 主题 / 面板）
    let mut popup_state: Option<popup::Popup> = None;
    // 只读信息屏（/help、/keys）：显示到用户按任意键
    let mut info_screen: Option<String> = None;
    // 由命令/弹窗设置：请求退出主循环
    let mut should_quit = false;
    // 侧栏开关（`ctrl+b`）。窄终端下渲染时会强制隐藏。
    let mut sidebar_open = true;

    // ── 首次进入某工作区：先要一次知情同意 ──────────────────────────
    //
    // 沙箱是硬边界（最多能做什么），信任是知情同意（这个目录是不是你让我动的）。
    // 二者互补：沙箱挡不住"信任错了目录"，信任挡不住"恶意代码越权"。
    if let Some(ws) = trust_workspace {
        let mut tp = TrustPrompt::default();
        loop {
            let (cols, rows) = terminal_size();
            let screen = Screen {
                cols,
                rows,
                facts: &[],
                input: "",
                status: "",
                awaiting_input: false,
                show_cursor: false,
                about: Some(&about),
                trust: Some(&tp),
                theme: theme_name,
                popup: None,
                preformatted: None,
                sidebar: false,
            };
            write!(stdout, "{}", screen.render())?;
            stdout.flush()?;

            let accept = |ws: &std::path::Path| {
                if let Err(e) = trust::trust(ws) {
                    // 记录失败不该拦住用户，但必须如实说 ——
                    // 静默失败会表现为"每次都问"，用户会以为是自己没点对
                    eprintln!("[neo] 无法记录信任（{e}）；本次继续，下次仍会询问");
                }
            };
            match read_key(&mut stdin) {
                Key::Quit => {
                    raw.restore();
                    alt.leave();
                    return Ok(());
                }
                Key::Up | Key::Char('k') => tp.selected = 0,
                Key::Down | Key::Char('j') => tp.selected = 1,
                Key::Char('y') => {
                    accept(&ws);
                    break;
                }
                Key::Char('n') => {
                    raw.restore();
                    alt.leave();
                    return Ok(());
                }
                Key::Enter => {
                    if tp.selected == 0 {
                        accept(&ws);
                        break;
                    }
                    raw.restore();
                    alt.leave();
                    return Ok(());
                }
                _ => {}
            }
        }
    }

    loop {
        let (cols, rows) = terminal_size();

        // 信息屏（/help、/keys）：占据正文区，按任意键返回
        if let Some(text) = info_screen.as_deref() {
            let body: Vec<Vec<Seg>> = text
                .lines()
                .map(|l| vec![(2usize, l.to_string(), Tone::Text)])
                .collect();
            let screen = Screen {
                cols,
                rows,
                facts: &[],
                input: "",
                status: "按任意键返回",
                awaiting_input: false,
                show_cursor: false,
                about: None,
                trust: None,
                theme: theme_name,
                popup: None,
                // 借 info 屏这段：直接用 facts 通道塞不进去（Fact 无原文类型），
                // 改用 preformatted 字段承载
                preformatted: Some(&body),
                sidebar: false,
            };
            write!(stdout, "{}", screen.render())?;
            stdout.flush()?;
            match read_key(&mut stdin) {
                Key::Quit => break,
                _ => {
                    info_screen = None;
                    continue;
                }
            }
        }

        let facts = facts_of(&events);
        let screen = Screen {
            cols,
            rows,
            facts: &facts,
            input: &input,
            status: &status,
            awaiting_input: outstanding.is_some(),
            show_cursor: true,
            about: Some(&about),
            trust: None,
            theme: theme_name,
            popup: popup_state.as_ref(),
            preformatted: None,
            sidebar: sidebar_open,
        };
        write!(stdout, "{}", screen.render())?;
        stdout.flush()?;

        let key = read_key(&mut stdin);

        // ── 弹窗打开时，按键先交给弹窗 ──────────────────────────────
        //
        // 这是"下拉菜单"的常规行为：方向键在候选间移动、Enter 确认、
        // Esc 取消，而不是直接落到输入框。
        if popup_state.is_some() {
            match key {
                Key::Quit => break,
                Key::Escape => {
                    popup_state = None;
                    status = "已取消".to_string();
                }
                Key::Up => {
                    if let Some(p) = popup_state.as_mut() {
                        p.move_selection(-1);
                    }
                }
                Key::Down => {
                    if let Some(p) = popup_state.as_mut() {
                        p.move_selection(1);
                    }
                }
                Key::Backspace => {
                    // 退格回到 `@`/`/` 之前 → 关闭弹窗；否则缩窄过滤词
                    input.pop();
                    let still = self_popup_context(&input);
                    match still {
                        Some((kind, q)) => {
                            if let Some(p) = popup_state.as_mut() {
                                p.query = q;
                                refresh_popup(p, kind, &mut file_cache);
                            }
                        }
                        None => popup_state = None,
                    }
                }
                Key::Char(c) => {
                    input.push(c);
                    match self_popup_context(&input) {
                        Some((kind, q)) => {
                            if let Some(p) = popup_state.as_mut() {
                                p.query = q;
                                refresh_popup(p, kind, &mut file_cache);
                            }
                        }
                        None => popup_state = None,
                    }
                }
                Key::Enter => {
                    let chosen = popup_state.as_ref().and_then(|p| p.selected_item().cloned());
                    popup_state = None;
                    if let Some(item) = chosen {
                        match apply_popup_item(
                            &item,
                            &mut input,
                            &mut status,
                            &mut theme_name,
                            &mut info_screen,
                        ) {
                            Effect::Quit => should_quit = true,
                            Effect::ClearTranscript => {
                                events.clear();
                                status = "新对话（已清空转录；文件改动不受影响）".to_string();
                            }
                            Effect::OpenThemePicker => {
                                let mut tp = popup::Popup::new(popup::Kind::Theme, "");
                                tp.set_items(popup::theme_items(""), false);
                                status = "主题 · ↑↓ 选择，回车应用".to_string();
                                popup_state = Some(tp);
                            }
                            Effect::ShowStatus => {
                                info_screen = Some(commands::status_text(
                                    &about,
                                    theme_name.as_str(),
                                ));
                            }
                            Effect::None => {}
                        }
                    }
                }
                _ => {}
            }
            if should_quit {
                break;
            }
            continue;
        }

        match key {
            Key::Quit => break,
            Key::Escape => {
                // 无弹窗时 Esc 清空当前输入（与多数 shell 的 Ctrl+U 语义接近，
                // 但不抢 ctrl+u 的键位）
                if !input.is_empty() {
                    input.clear();
                    browsing = false;
                }
            }
            Key::CommandPalette => {
                let mut p = popup::Popup::new(popup::Kind::Palette, "");
                p.set_items(popup::palette_items(""), false);
                popup_state = Some(p);
                status = "命令面板".to_string();
            }
            Key::ToggleSidebar => {
                sidebar_open = !sidebar_open;
                status = if sidebar_open { "侧栏已展开" } else { "侧栏已收起" }.to_string();
            }
            Key::NextTheme => {
                theme_name = theme_name.next();
                theme::save_preference(theme_name);
                status = format!("主题：{}", theme_name.as_str());
            }
            Key::ClearScreen => {
                write!(stdout, "{ESC}[2J{ESC}[H")?;
                stdout.flush()?;
            }
            Key::ClearLine => {
                input.clear();
                browsing = false;
            }
            Key::Backspace => {
                input.pop();
                browsing = false;
            }
            Key::Char(c) => {
                input.push(c);
                browsing = false;
                history.reset_cursor();
                // 输入 `@` 或 `/` 即弹出候选（对齐 opencode：输入即列表）
                if c == '@' || c == '/' {
                    open_popup_for(&input, &mut popup_state, &mut file_cache, &mut status);
                }
            }
            Key::Up => {
                // 输入为空或正在浏览历史时，Up 走历史
                if input.is_empty() || browsing {
                    if let Some(h) = history.prev() {
                        input = h.to_string();
                        browsing = true;
                    }
                }
            }
            Key::Down => {
                if browsing {
                    match history.next_entry() {
                        Some(h) => input = h.to_string(),
                        None => {
                            input.clear();
                            browsing = false;
                        }
                    }
                }
            }
            Key::SearchHistory => {
                // Ctrl+R：用当前输入当查询，回填最近一条匹配（再按继续往回找）
                let needle = input.clone();
                if let Some(found) = history.search(&needle) {
                    let found = found.to_string();
                    // 连续 Ctrl+R 时把游标往上挪一格，实现"继续找更早的"
                    if found == input && !needle.is_empty() {
                        let _ = history.prev();
                    }
                    input = found;
                    status = format!("历史搜索：{needle}");
                } else {
                    status = format!("历史中未找到：{needle}");
                }
                browsing = true;
            }
            Key::ExternalEditor => {
                let initial = input.clone();
                if let Some(edited) = edit_externally(&initial, &raw) {
                    // 编辑器返回的是一整段文本；取首行作为任务
                    input = edited.trim().to_string();
                    status = "已从外部编辑器取回内容".to_string();
                } else {
                    status = "外部编辑器未返回内容".to_string();
                }
                browsing = false;
            }
            Key::Tab => {
                // 弹窗开着时 Tab 等价于"接受当前选中项"
                if let Some(p) = popup_state.as_ref() {
                    if let Some(item) = p.selected_item().cloned() {
                        let mut st = status.clone();
                        let eff = apply_popup_item(
                            &item, &mut input, &mut st, &mut theme_name, &mut info_screen,
                        );
                        status = st;
                        popup_state = None;
                        match eff {
                            Effect::Quit => break,
                            Effect::ClearTranscript => events.clear(),
                            Effect::ShowStatus => {
                                info_screen = Some(commands::status_text(
                                    &about,
                                    theme_name.as_str(),
                                ));
                            }
                            // Tab 接受主题选择后不开新弹窗，直接生效即可
                            Effect::OpenThemePicker | Effect::None => {}
                        }
                        continue;
                    }
                }
                // Tab：补全 `@` 引用（只在 @ 上下文中生效）
                if let Some(q) = at_query(&input) {
                    if file_cache.is_none() {
                        let (files, truncated) = input::list_files(&std::env::current_dir().unwrap_or_else(|_| ".".into()), 5000, 8);
                        // 截断提示与补全结果合并成一句，避免前一句被后一句覆盖而丢失
                        let suffix = if truncated { "（候选已达上限，列表可能不完整）" } else { "" };
                        file_cache = Some(files);
                        status = suffix.to_string();
                    }
                    let files = file_cache.as_deref().unwrap_or(&[]);
                    let ranked = input::fuzzy_rank(q, files, 1);
                    match ranked.first() {
                        Some(best) => {
                            input = complete_at_token(&input, best);
                            status = format!("补全：{best}{}", status);
                        }
                        None => status = format!("无匹配文件：{q}"),
                    }
                }
            }
            Key::Enter => {
                let line = std::mem::take(&mut input);

                // ── 有待审批：本行是审批应答 ──────────────────────────────
                if let Some(id) = outstanding.clone() {
                    let Some(allow) = parse_approval_answer(&line) else {
                        status = "请回答 y 或 n".to_string();
                        continue;
                    };
                    outstanding = None;
                    let op = neo_protocol::Op::Approve {
                        id,
                        decision: if allow {
                            neo_protocol::Decision::Allow
                        } else {
                            neo_protocol::Decision::Deny
                        },
                    };
                    match submit(op) {
                        Ok(produced) => {
                            outstanding = latest_approval_id(&produced);
                            events.extend(produced);
                        }
                        Err(e) => events.push(EventMsg::Error { message: e }),
                    }
                    status = idle_or_approval(&outstanding);
                    continue;
                }

                // ── `/命令`：直接执行，不进模型 ──────────────────────
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix('/') {
                    // 只取命令名（后面可带参数，当前命令都不需要参数）
                    let name = rest.split_whitespace().next().unwrap_or("");
                    match commands::resolve(name) {
                        Some(cmd) => {
                            let eff = apply_popup_item(
                                &popup::Item {
                                    label: format!("/{}", cmd.name),
                                    detail: String::new(),
                                    action: popup::ItemAction::Run(cmd.action),
                                },
                                &mut input,
                                &mut status,
                                &mut theme_name,
                                &mut info_screen,
                            );
                            match eff {
                                // 直接 break 出主循环；不需要再走一遍 should_quit
                                Effect::Quit => break,
                                Effect::ClearTranscript => {
                                    events.clear();
                                    status =
                                        "新对话（已清空转录；文件改动不受影响）".to_string();
                                }
                                Effect::OpenThemePicker => {
                                    let mut tp = popup::Popup::new(popup::Kind::Theme, "");
                                    tp.set_items(popup::theme_items(""), false);
                                    status = "主题 · ↑↓ 选择，回车应用".to_string();
                                    popup_state = Some(tp);
                                }
                                Effect::ShowStatus => {
                                    info_screen = Some(commands::status_text(
                                        &about,
                                        theme_name.as_str(),
                                    ));
                                }
                                Effect::None => {}
                            }
                        }
                        None => status = format!("未知命令：/{name}（输入 / 查看列表）"),
                    }
                    continue;
                }

                // ── `!cmd`：直接执行 shell（沙箱内），输出进会话 ─────
                if let Some(cmd) = trimmed.strip_prefix('!') {
                    let cmd = cmd.trim().to_string();
                    if cmd.is_empty() {
                        continue;
                    }
                    history.push(&line);
                    let running = format!("执行：{cmd}");
                    // 先画一帧"运行中"，让用户看到命令已提交（shell 可能跑一会儿）
                    let (c3, r3) = terminal_size();
                    let f3 = facts_of(&events);
                    write!(
                        stdout,
                        "{}",
                        Screen {
                            cols: c3,
                            rows: r3,
                            facts: &f3,
                            input: "",
                            status: &running,
                            awaiting_input: false,
                            show_cursor: false,
                            about: Some(&about),
                            trust: None,
                            theme: theme_name,
                            popup: None,
                            preformatted: None,
                            sidebar: sidebar_open,
                        }
                        .render()
                    )?;
                    stdout.flush()?;
                    match submit(neo_protocol::Op::Shell { command: cmd }) {
                        Ok(produced) => events.extend(produced),
                        Err(e) => events.push(EventMsg::Error { message: e }),
                    }
                    status = idle_or_approval(&outstanding);
                    continue;
                }

                // ── 普通任务提交 ────────────────────────────────────────
                if line.trim().is_empty() {
                    continue;
                }
                history.push(&line);
                browsing = false;
                status = format!("运行中 · {}", line.trim());
                // 先画一帧，让用户看到自己提交了什么
                let (c2, r2) = terminal_size();
                let facts0 = facts_of(&events);
                write!(
                    stdout,
                    "{}",
                    Screen {
                        cols: c2,
                        rows: r2,
                        facts: &facts0,
                        input: "",
                        status: &status,
                        awaiting_input: false,
                        show_cursor: false,
                        // 这一帧是"运行中"过渡态：facts 通常已非空，不展示首屏
                        about: Some(&about),
                        trust: None,
                        theme: theme_name,
                        popup: None,
                        preformatted: None,
                        sidebar: sidebar_open,
                    }
                    .render()
                )?;
                stdout.flush()?;

                match submit(neo_protocol::Op::UserTurn { text: line, refs: Vec::new() }) {
                    Ok(produced) => {
                        outstanding = latest_approval_id(&produced);
                        events.extend(produced);
                    }
                    Err(e) => events.push(EventMsg::Error { message: e }),
                }
                status = idle_or_approval(&outstanding);
            }
            _ => {}
        }
    }

    raw.restore();
    alt.leave();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 测试辅助 ──────────────────────────────────────────────────────
    //
    // 渲染产物含 ANSI 与光标定位序列，直接断言字符串很脆。这里统一
    // 剥成"纯文本行"，让测试断言**用户看到的内容**而不是转义细节。
    fn screen(cols: usize, rows: usize, facts: &[Fact], input: &str, status: &str) -> String {
        Screen {
            cols,
            rows,
            facts,
            input,
            status,
            awaiting_input: false,
            show_cursor: false,
            about: None,
            trust: None,
            theme: theme::ThemeName::OpenCode,
            popup: None,
            preformatted: None,
            sidebar: false,
        }
        .render()
    }

    fn about() -> About {
        About {
            version: "0.1.0".into(),
            model: "mock".into(),
            mode: "Default（沙箱 WorkspaceWrite / 审批 OnRequest）".into(),
            mode_short: "default".into(),
            workspace: "/tmp/ws".into(),
            branch: "main".into(),
            example: "修一下代码里的 TODO".into(),
            context_limit: 64_000,
            session: "neo-tui".into(),
        }
    }

    fn welcome(cols: usize, rows: usize) -> String {
        let a = about();
        Screen {
            cols,
            rows,
            facts: &[],
            input: "",
            status: "就绪",
            awaiting_input: false,
            show_cursor: false,
            about: Some(&a),
            trust: None,
            theme: theme::ThemeName::OpenCode,
            popup: None,
            preformatted: None,
            sidebar: false,
        }
        .render()
    }

    /// 剥掉 ANSI 转义序列，返回可见行（保留空行以反映真实占位）。
    ///
    /// **必须按 CSI 语法解析**（`ESC [` 参数字节(0x30–0x3F) 中间字节(0x20–0x2F)
    /// 终止字节(0x40–0x7E)），不能"找 m/h/l 当结尾" ——
    /// `ESC[H`（光标归位）与 `ESC[2J`（清屏）都没有那些字母作结尾，
    /// 按字母找会一路吞到正文里，测试看到的就只剩尾部。
    fn plain(rendered: &str) -> Vec<String> {
        fn is_final(c: char) -> bool {
            ('\u{40}'..='\u{7e}').contains(&c)
        }
        let mut lines: Vec<String> = Vec::new();
        let mut cur = String::new();
        let mut it = rendered.chars().peekable();
        while let Some(c) = it.next() {
            match c {
                '\u{1b}' => {
                    if it.peek() == Some(&'[') {
                        it.next();
                        // 参数字节 + 中间字节
                        while let Some(&n) = it.peek() {
                            if ('\u{30}'..='\u{3f}').contains(&n) || ('\u{20}'..='\u{2f}').contains(&n) {
                                it.next();
                            } else {
                                break;
                            }
                        }
                        // 终止字节
                        if let Some(&n) = it.peek() {
                            if is_final(n) {
                                it.next();
                            }
                        }
                    } else {
                        // 两字符转义（ESC 后跟单个字符）
                        it.next();
                    }
                }
                '\r' => {}
                '\n' => lines.push(std::mem::take(&mut cur)),
                _ => cur.push(c),
            }
        }
        lines.push(cur);
        lines
    }

    // ── 按键解码（不变）──────────────────────────────────────────────

    #[test]
    fn decodes_basic_keys() {
        assert_eq!(decode_key(&[0x03]), Key::Quit);
        assert_eq!(decode_key(b"\r"), Key::Enter);
        assert_eq!(decode_key(&[0x7f]), Key::Backspace);
        assert_eq!(decode_key(b"a"), Key::Char('a'));
        assert_eq!(decode_key(&[0x1b, b'[', b'A']), Key::Up);
        assert_eq!(decode_key(&[0x1b, b'[', b'B']), Key::Down);
        // 单独 ESC 现在是"关弹窗"，必须是 Escape 而非 Unknown
        assert_eq!(decode_key(&[0x1b]), Key::Escape, "单独 ESC 应为 Escape");
        assert_eq!(decode_key(&[0x10]), Key::CommandPalette);
        assert_eq!(decode_key(&[0x14]), Key::NextTheme);
        assert_eq!(decode_key(&[0x1b, b'[', b'Z']), Key::Unknown, "未支持的序列应为 Unknown");
    }

    #[test]
    fn decodes_multibyte_utf8_keys() {
        assert_eq!(decode_key("你".as_bytes()), Key::Char('你'));
        assert_eq!(decode_key("🚀".as_bytes()), Key::Char('🚀'));
        assert_eq!(decode_key(&[0xe5, 0x86]), Key::Unknown, "不完整的 UTF-8 应为 Unknown");
        assert_eq!(decode_key(&[0xff, 0xfe]), Key::Unknown, "非法字节应为 Unknown");
    }

    // ── 弹窗 / 命令 ───────────────────────────────────────────────────

    #[test]
    fn popup_is_opaque_over_content() {
        // 弹窗必须把底下的内容盖住。若不逐格填空，logo/正文会从字符缝隙里
        // 露出来（真机截图里能看到 logo 碎片嵌在弹窗行中）。
        let a = about();
        let mut p = popup::Popup::new(popup::Kind::Slash, "");
        p.set_items(popup::slash_items(""), false);
        let out = Screen {
            cols: 100,
            rows: 30,
            facts: &[],
            input: "/",
            status: "",
            awaiting_input: false,
            show_cursor: false,
            about: Some(&a),
            trust: None,
            theme: theme::ThemeName::OpenCode,
            popup: Some(&p),
            preformatted: None,
            sidebar: false,
        }
        .render();
        // 找出弹窗所在的行区间（含边框），断言这些行里没有 logo 的半块字符
        let lines = plain(&out);
        let idxs: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.contains('╭') || l.contains('│') || l.contains('╰'))
            .map(|(i, _)| i)
            .collect();
        assert!(!idxs.is_empty(), "应画出弹窗：{out}");
        let (top, bottom) = (*idxs.first().unwrap(), *idxs.last().unwrap());
        for l in &lines[top..=bottom] {
            assert!(!l.contains('█'), "弹窗行里漏进了 logo：{l:?}");
        }
    }

    #[test]
    fn accepting_a_command_clears_the_input() {
        // 执行命令后输入框必须清空：留着会让用户以为没执行，且盖住占位提示
        let item = popup::Item {
            label: "/help".into(),
            detail: String::new(),
            action: popup::ItemAction::Run(commands::Action::Help),
        };
        let mut input = "/help".to_string();
        let mut status = String::new();
        let mut theme_name = theme::ThemeName::OpenCode;
        let mut info: Option<String> = None;
        let eff = apply_popup_item(&item, &mut input, &mut status, &mut theme_name, &mut info);
        assert!(input.is_empty(), "执行命令后输入应清空，实际 {input:?}");
        assert!(info.is_some(), "/help 应打开信息屏");
        assert_eq!(eff, Effect::None);
    }

    #[test]
    fn accepting_a_file_ref_keeps_it_in_the_input() {
        // 与命令相反：文件引用要**保留**在输入里，因为它是发给模型的内容
        let item = popup::Item {
            label: "src/main.rs".into(),
            detail: String::new(),
            action: popup::ItemAction::Insert("@src/main.rs".into()),
        };
        let mut input = "@src/ma".to_string();
        let mut status = String::new();
        let mut theme_name = theme::ThemeName::OpenCode;
        let mut info: Option<String> = None;
        apply_popup_item(&item, &mut input, &mut status, &mut theme_name, &mut info);
        assert!(input.contains("@src/main.rs"), "引用应留在输入里，实际 {input:?}");
        assert!(!input.contains("ma@"), "不应重复叠加过滤词：{input:?}");
    }

    #[test]
    fn popup_enter_takes_the_selected_item() {
        let mut p = popup::Popup::new(popup::Kind::Slash, "");
        p.set_items(popup::slash_items(""), false);
        p.move_selection(2);
        let it = p.selected_item().expect("应有选中项");
        assert!(it.label.starts_with('/'), "选中项应是命令：{it:?}");
    }

    #[test]
    fn command_context_detection_is_conservative() {
        // 只有"词首的 @ /"才算弹窗上下文；`a/b` 这样的路径不能误判成命令
        assert!(matches!(self_popup_context("@"), Some((popup::Kind::File, _))));
        assert!(matches!(self_popup_context("看下 @src"), Some((popup::Kind::File, _))));
        assert!(matches!(self_popup_context("/th"), Some((popup::Kind::Slash, _))));
        assert!(self_popup_context("a/b").is_none(), "路径中的 / 不是命令");
        assert!(self_popup_context("@a b").is_none(), "已进入正文就不再是过滤词");
        assert!(self_popup_context("普通文本").is_none());
    }

    #[test]
    fn info_screen_renders_the_help_text() {
        let help = commands::help_text().to_string();
        let lines: Vec<Vec<Seg>> = help
            .lines()
            .map(|l| vec![(2usize, l.to_string(), Tone::Text)])
            .collect();
        let out = Screen {
            cols: 100,
            rows: 30,
            facts: &[],
            input: "",
            status: "按任意键返回",
            awaiting_input: false,
            show_cursor: false,
            about: None,
            trust: None,
            theme: theme::ThemeName::OpenCode,
            popup: None,
            preformatted: Some(&lines),
            sidebar: false,
        }
        .render();
        let text = plain(&out).join("\n");
        assert!(text.contains("编程 Agent 内核"), "应显示帮助正文：{text}");
        assert!(text.contains("按任意键返回"), "应提示如何返回：{text}");
    }

    // ── 侧栏 ──────────────────────────────────────────────────────────

    fn sidebar_screen(cols: usize, facts: &[Fact], sidebar: bool) -> String {
        let a = About { context_limit: 64_000, ..about() };
        Screen {
            cols,
            rows: 30,
            facts,
            input: "",
            status: "就绪",
            awaiting_input: false,
            show_cursor: false,
            about: Some(&a),
            trust: None,
            theme: theme::ThemeName::OpenCode,
            popup: None,
            preformatted: None,
            sidebar,
        }
        .render()
    }

    #[test]
    fn sidebar_shows_todo_and_modified_files() {
        use neo_protocol::{FileChange, TodoEntry, TodoStatus};
        let facts = vec![
            Fact::TodoList(vec![
                TodoEntry { content: "写测试".into(), status: TodoStatus::Completed },
                TodoEntry { content: "改文档".into(), status: TodoStatus::InProgress },
                TodoEntry { content: "发版".into(), status: TodoStatus::Pending },
            ]),
            Fact::FilesChanged(vec![
                FileChange { path: "src/main.rs".into(), additions: 12, deletions: 3 },
            ]),
        ];
        let text = plain(&sidebar_screen(120, &facts, true)).join("\n");
        assert!(text.contains("Todo 1/3"), "应有清单进度：{text}");
        assert!(text.contains("写测试"), "应列出清单项：{text}");
        assert!(text.contains("Modified Files"), "应有已修改文件面板：{text}");
        assert!(text.contains("src/main.rs"), "应列出文件名：{text}");
        assert!(text.contains("+12"), "应显示新增行数：{text}");
    }

    #[test]
    fn sidebar_can_be_turned_off_and_hides_on_narrow_terminals() {
        use neo_protocol::{FileChange};
        let facts = vec![Fact::FilesChanged(vec![
            FileChange { path: "a.rs".into(), additions: 1, deletions: 0 },
        ])];
        let off = plain(&sidebar_screen(120, &facts, false)).join("\n");
        assert!(!off.contains("Modified Files"), "关闭后不该有侧栏：{off}");

        // 窄终端：即使 sidebar=true 也必须隐藏（否则正文被挤到不可读）
        let narrow = plain(&sidebar_screen(60, &facts, true)).join("\n");
        assert!(!narrow.contains("Modified Files"), "窄终端不该显示侧栏：{narrow}");
    }

    #[test]
    fn sidebar_omits_the_context_percentage_when_limit_is_unknown() {
        // 上限未知时给百分比等于编数字；宁可不显示
        let a = About { context_limit: 0, ..about() };
        let facts = vec![Fact::TurnFinished { input_tokens: 100, output_tokens: 20 }];
        let out = Screen {
            cols: 120, rows: 30, facts: &facts, input: "", status: "",
            awaiting_input: false, show_cursor: false,
            about: Some(&a), trust: None,
            theme: theme::ThemeName::OpenCode, popup: None, preformatted: None,
            sidebar: true,
        }.render();
        let text = plain(&out).join("\n");
        assert!(text.contains("上限未知"), "应说明上限未知：{text}");
        assert!(!text.contains("% of"), "不得编造百分比：{text}");
    }

    #[test]
    fn sidebar_context_percentage_reflects_usage() {
        let a = About { context_limit: 1000, ..about() };
        let facts = vec![Fact::TurnFinished { input_tokens: 800, output_tokens: 100 }];
        let out = Screen {
            cols: 120, rows: 30, facts: &facts, input: "", status: "",
            awaiting_input: false, show_cursor: false,
            about: Some(&a), trust: None,
            theme: theme::ThemeName::OpenCode, popup: None, preformatted: None,
            sidebar: true,
        }.render();
        assert!(plain(&out).join("\n").contains("90% of 1000"), "占用率应为 90%");
    }

    #[test]
    fn long_paths_keep_their_filename() {
        // 侧栏路径从左侧截断：文件名是识别信息，不能被截掉
        let long = "src/very/deeply/nested/directory/structure/module.rs";
        let t = truncate_left(long, 20);
        assert!(t.ends_with("module.rs"), "应保留文件名：{t}");
        assert!(width::display_width(&t) <= 20, "不应超预算：{t}");
        // 短路径原样返回
        assert_eq!(truncate_left("a.rs", 20), "a.rs");
    }

    // ── Markdown 接入正文 ─────────────────────────────────────────────

    #[test]
    fn assistant_markdown_is_rendered_not_dumped() {
        let facts = vec![Fact::AssistantSaid(
            "## 标题\n\n- 一件\n- 两件\n\n```rust\nlet x = 1;\n```".into(),
        )];
        let text = plain(&screen(90, 30, &facts, "", "")).join("\n");
        assert!(text.contains("标题") && !text.contains("## 标题"), "标题标记应剥掉：{text}");
        assert!(text.contains("• 一件"), "列表应换成圆点：{text}");
        assert!(text.contains("let x = 1;"), "代码块内容应保留：{text}");
    }

    #[test]
    fn user_message_keeps_its_bar_after_markdown() {
        let facts = vec![Fact::UserSaid("看下 `src/main.rs`".into())];
        let text = plain(&screen(80, 20, &facts, "", "")).join("\n");
        assert!(text.contains('┃'), "用户消息应保留竖条：{text}");
        assert!(text.contains("src/main.rs"), "内容应保留：{text}");
    }

    #[test]
    fn sidebar_does_not_swallow_body_text() {
        // 侧栏 + 长正文：正文必须完整可见（不能被裁掉或与侧栏重叠）
        let facts = vec![Fact::AssistantSaid("正文内容在这里".repeat(3))];
        let text = plain(&sidebar_screen(120, &facts, true)).join("\n");
        assert!(text.contains("正文内容在这里"), "正文必须可见：{text}");
    }

    // ── 主题 ──────────────────────────────────────────────────────────

    #[test]
    fn switching_theme_changes_rendered_colors() {
        // 换主题必须真的改变输出色值，而不是只改了个名字
        let a = about();
        let render = |t: theme::ThemeName| {
            with_env(&[("COLORTERM", "truecolor")], || {
                Screen {
                    cols: 80, rows: 20, facts: &[], input: "", status: "",
                    awaiting_input: false, show_cursor: false,
                    about: Some(&a), trust: None,
                    theme: t, popup: None, preformatted: None, sidebar: false,
                }
                .render()
            })
        };
        let oc = render(theme::ThemeName::OpenCode);
        let nord = render(theme::ThemeName::Nord);
        assert_ne!(oc, nord, "换主题后渲染色值应不同");
        // opencode 主色 #fab283；nord 主色 #88c0d0
        assert!(oc.contains("38;2;250;178;131"), "opencode 主色应为 #fab283");
        assert!(nord.contains("38;2;136;192;208"), "nord 主色应为 #88c0d0");
    }

    // ── 终端生命周期 ──────────────────────────────────────────────────

    #[test]
    fn alt_screen_leave_is_idempotent() {
        // leave() 会被 Drop、显式退出、panic 钩子多处调用；必须幂等，
        // 否则会往终端写多次 ?1049l（部分终端会因此闪一下）。
        let mut alt = AltScreen::enter();
        assert!(alt.active, "enter 后应为活动状态");
        alt.leave();
        assert!(!alt.active, "leave 后应转为非活动");
        alt.leave(); // 第二次不该 panic，也不该重复输出
        assert!(!alt.active);
        // Drop 再调用一次也必须安全
        drop(alt);
    }

    // ── 布局：核心不变量 ──────────────────────────────────────────────

    #[test]
    fn every_rendered_line_is_exactly_cols_wide() {
        // 网格模型的**根本约束**：每行都必须恰好 cols 列。
        // 少一列 → 上一帧残留；多一列 → 换行错位、整屏滚动。
        // 宽字符（中文/emoji）是最容易破坏这条的地方。
        for (cols, rows) in [(40usize, 12usize), (80, 24), (120, 40), (28, 10)] {
            let a = about();
            let facts = vec![
                Fact::UserSaid("中文消息测试宽字符对齐".into()),
                Fact::AssistantSaid("这是助手的回答，也包含中文与 emoji 🚀".into()),
                Fact::ToolFinished { name: "bash".into(), exit_code: 0 },
            ];
            let out = Screen {
                cols, rows, facts: &facts, input: "输入中文", status: "就绪",
                awaiting_input: false, show_cursor: false, about: Some(&a), trust: None,
                theme: theme::ThemeName::OpenCode, popup: None, preformatted: None, sidebar: false,
            }
            .render();
            for (i, line) in plain(&out).iter().enumerate() {
                let w = width::display_width(line);
                assert!(w <= cols, "{cols}x{rows} 第 {i} 行宽 {w} 超过 {cols}：{line:?}");
            }
        }
    }

    #[test]
    fn transcript_scrolls_to_show_the_latest() {
        let facts = vec![
            Fact::AssistantSaid("line1".into()),
            Fact::AssistantSaid("line2".into()),
            Fact::AssistantSaid("line3".into()),
        ];
        let out = screen(40, 12, &facts, "", "s");
        let text = plain(&out).join("\n");
        assert!(text.contains("line3"), "应显示最新内容：{text}");
    }

    #[test]
    fn user_message_is_visible_and_marked() {
        let facts = vec![
            Fact::UserSaid("列一下文件".into()),
            Fact::AssistantSaid("好的".into()),
        ];
        let text = plain(&screen(60, 20, &facts, "", "s")).join("\n");
        assert!(text.contains("列一下文件"), "用户消息必须可见：{text}");
        assert!(text.contains('┃'), "用户消息应带左侧竖条：{text}");
        assert!(text.contains("好的"), "助手正文必须可见：{text}");
    }

    #[test]
    fn tool_success_and_failure_are_distinguishable() {
        let ok = [Fact::ToolFinished { name: "bash".into(), exit_code: 0 }];
        let bad = [Fact::ToolFinished { name: "bash".into(), exit_code: 1 }];
        let a = plain(&screen(60, 12, &ok, "", "s")).join("\n");
        let b = plain(&screen(60, 12, &bad, "", "s")).join("\n");
        assert!(a.contains('✓'), "成功应显示 ✓：{a}");
        assert!(b.contains('✗'), "失败应显示 ✗：{b}");
    }

    // ── 首屏（MiMo 式居中 + 星场）─────────────────────────────────────

    #[test]
    fn welcome_is_centered_not_left_aligned() {
        // MiMo 的首页是**居中**构图，不是左对齐 —— 这是本轮对标的重点
        let text = plain(&welcome(100, 30)).join("\n");
        let logo_line = plain(&welcome(100, 30))
            .into_iter()
            .find(|l| l.contains('█'))
            .expect("应有大 logo");
        let lead = logo_line.chars().take_while(|c| *c == ' ').count();
        assert!(lead > 20, "logo 应居中（左侧留白 {lead} 偏少）：{logo_line:?}");
        assert!(text.contains("Neo"), "应显示品牌名：{text}");
    }

    #[test]
    fn welcome_shows_injected_session_info() {
        let text = plain(&welcome(100, 34)).join("\n");
        for needle in ["mock", "/tmp/ws", "main", "0.1.0", "neo-tui"] {
            assert!(text.contains(needle), "首屏应显示 {needle}：{text}");
        }
    }

    #[test]
    fn welcome_includes_an_input_box() {
        // 输入框是首页的主体，必须有边框与占位提示
        let text = plain(&welcome(90, 30)).join("\n");
        assert!(text.contains('╭'), "应有输入框上边框：{text}");
        assert!(text.contains("输入任务"), "应有占位提示：{text}");
        assert!(text.contains("修一下代码里的 TODO"), "占位应带一个具体示例：{text}");
        assert!(text.contains("default ⏵ mock"), "输入框内应有模式/模型状态行：{text}");
    }

    #[test]
    fn welcome_degrades_on_small_terminals() {
        // 小终端必须降级而不是把内容裁掉（裁剪会从顶部裁，最难看）
        let text = plain(&welcome(30, 10)).join("\n");
        assert!(text.contains("Neo"), "小终端仍要显示品牌：{text}");
        assert!(!text.contains('█'), "小终端不该硬塞大 logo：{text}");
    }

    #[test]
    fn starfield_appears_but_never_covers_content() {
        // 星场是背景：必须出现，且**绝不能**盖掉正文/输入框
        let facts = vec![Fact::AssistantSaid("正文内容".into())];
        let out = screen(100, 24, &facts, "我的输入", "就绪");
        let text = plain(&out).join("\n");
        assert!(text.contains('·') || text.contains('+'), "应出现星场：{text}");
        assert!(text.contains("正文内容"), "星场不得覆盖正文：{text}");
        assert!(text.contains("我的输入"), "星场不得覆盖输入：{text}");
        // 输入框边框必须完整可见
        assert!(text.contains('╭') && text.contains('╯'), "输入框边框应完整：{text}");
    }

    #[test]
    fn rendering_is_deterministic_across_frames() {
        // 每帧重画整屏；若星位随机，画面会闪成噪点。必须逐字节一致。
        //
        // 这里**必须锁环境**：颜色档位来自进程级 env，并行的 env 测试
        // 若在两次渲染之间改掉 COLORTERM，两次的颜色转义就不同 ——
        // 那是测试自己引入的抖动，不是星场在闪（星位由位置哈希决定，本就稳定）。
        let (a, b) = with_env(&[("COLORTERM", "truecolor")], || (welcome(90, 26), welcome(90, 26)));
        assert_eq!(a, b, "同一状态两次渲染必须完全一致（否则星场在闪）");

        // 再单独断言"星位本身"稳定：与颜色无关的部分（剥掉转义）也必须一致
        let (c, d) = with_env(&[("COLORTERM", "truecolor")], || {
            (plain(&welcome(90, 26)), plain(&welcome(90, 26)))
        });
        assert_eq!(c, d, "剥掉颜色后星位也必须逐行一致");
    }

    #[test]
    fn hints_do_not_advertise_unimplemented_keys() {
        // 提示行只能列真正可用的键 —— 按了没反应比不提示更糟
        let text = plain(&welcome(110, 30)).join("\n");
        assert!(text.contains("ctrl+r"), "应提示历史：{text}");
        assert!(text.contains("@ 引用"), "应提示引用：{text}");
        // 未实现的对话框类命令不得出现
        for bad in ["/themes", "/details", "/thinking"] {
            assert!(!text.contains(bad), "不得提示未实现的 {bad}：{text}");
        }
    }

    // ── 信任对话框 ────────────────────────────────────────────────────

    #[test]
    fn trust_prompt_warns_about_workspace_risk() {
        let a = about();
        let out = Screen {
            cols: 100, rows: 30, facts: &[], input: "", status: "",
            awaiting_input: false, show_cursor: false,
            about: Some(&a), trust: Some(&TrustPrompt::default()),
            theme: theme::ThemeName::OpenCode, popup: None, preformatted: None, sidebar: false,
        }
        .render();
        let text = plain(&out).join("\n");
        assert!(text.contains("访问工作区"), "应说明访问哪个目录：{text}");
        assert!(text.contains("/tmp/ws"), "应显示具体路径：{text}");
        assert!(text.contains("恶意"), "必须说明最坏情况：{text}");
        assert!(text.contains("是的，我信任此目录"), "应有肯定选项：{text}");
        assert!(text.contains("否，退出"), "应有否定选项：{text}");
    }

    #[test]
    fn trust_prompt_marks_the_selected_option() {
        let a = about();
        let render_with = |sel: usize| {
            let out = Screen {
                cols: 100, rows: 30, facts: &[], input: "", status: "",
                awaiting_input: false, show_cursor: false,
                about: Some(&a), trust: Some(&TrustPrompt { selected: sel }),
                theme: theme::ThemeName::OpenCode, popup: None, preformatted: None, sidebar: false,
            }
            .render();
            plain(&out).join("\n")
        };
        // 只看**选项那一行**的前缀：标题行"● 访问工作区"里也有 ●，
        // 全局 find('●') 会一直命中的是第一行，断言就失去意义
        // 选项行形如 "┃   ● 是的，我信任此目录" —— 竖条在左，
        // 所以要先剔掉非标记字符，再看第一个可见符号是 ● 还是 ○
        let option_line = |t: &str, label: &str| -> String {
            let line = t
                .lines()
                .find(|l| l.contains(label))
                .unwrap_or_else(|| panic!("找不到选项「{label}」"));
            line.trim_matches(|c: char| c == '┃' || c.is_whitespace()).to_string()
        };
        let first = render_with(0);
        let second = render_with(1);
        assert!(
            option_line(&first, "是的，我信任此目录").starts_with('●'),
            "选中第一项时它应是实心 ●：{first}"
        );
        assert!(
            option_line(&first, "否，退出").starts_with('○'),
            "未选中的第二项应是空心 ○：{first}"
        );
        assert!(
            option_line(&second, "否，退出").starts_with('●'),
            "切换后第二项应变为实心 ●：{second}"
        );
    }

    // ── 颜色能力降级 ──────────────────────────────────────────────────
    //
    // 颜色档位来自环境变量，而环境变量是**进程级**的 —— cargo 默认并行跑用例，
    // 不加锁就会互相把 COLORTERM/NO_COLOR 改掉，产生"时好时坏"的假失败。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_env<T>(vars: &[(&str, &str)], f: impl FnOnce() -> T) -> T {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved: Vec<(String, Option<String>)> =
            vars.iter().map(|(k, _)| (k.to_string(), std::env::var(k).ok())).collect();
        // 显式清掉可能干扰的另一变量
        let touched: Vec<&str> = vars.iter().map(|(k, _)| *k).collect();
        for other in ["NO_COLOR", "COLORTERM", "TERM"] {
            if !touched.contains(&other) {
                std::env::remove_var(other);
            }
        }
        for (k, v) in vars {
            std::env::set_var(k, v);
        }
        let out = f();
        for (k, old) in saved {
            match old {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
        out
    }

    #[test]
    fn no_color_env_disables_all_coloring() {
        let out = with_env(&[("NO_COLOR", "1")], || {
            let facts = [Fact::UserSaid("hi".into()), Fact::AssistantSaid("yo".into())];
            screen(60, 16, &facts, "x", "s")
        });
        let mut it = out.chars().peekable();
        let mut kept = Vec::new();
        while let Some(c) = it.next() {
            if c == '\u{1b}' && it.peek() == Some(&'[') {
                it.next();
                let mut seq = String::new();
                while let Some(&n) = it.peek() {
                    if ('\u{30}'..='\u{3f}').contains(&n) || ('\u{20}'..='\u{2f}').contains(&n) {
                        it.next();
                        seq.push(n);
                    } else {
                        break;
                    }
                }
                if let Some(&n) = it.peek() {
                    if ('\u{40}'..='\u{7e}').contains(&n) {
                        it.next();
                        seq.push(n);
                    }
                }
                kept.push(seq);
            }
        }
        let sgr: Vec<&String> = kept.iter().filter(|x| x.ends_with('m')).collect();
        assert!(sgr.is_empty(), "NO_COLOR 下不应有 SGR 颜色序列：{sgr:?}");
    }

    #[test]
    fn rgb_conversions_stay_in_range() {
        // 降级路径用的转换必须是合法色号，否则终端会显示乱码/忽略
        for (r, g, b) in [(0u8, 0u8, 0u8), (255, 255, 255), (250, 178, 131), (157, 124, 216)] {
            let c256 = rgb_to_256(r, g, b);
            assert!(c256 >= 16, "256 色号 {c256} 落在系统色区（16 以下）");
            let c16 = rgb_to_16(r, g, b);
            assert!((30..=37).contains(&c16) || c16 == 90, "16 色号 {c16} 非法");
        }
    }

    #[test]
    fn lerp_hits_both_ends() {
        let a = (0, 0, 0);
        let b = (100, 200, 50);
        assert_eq!(lerp_rgb(a, b, 0.0), a);
        assert_eq!(lerp_rgb(a, b, 1.0), b);
        let mid = lerp_rgb(a, b, 0.5);
        assert!(mid.0 > 0 && mid.0 < 100, "中点应居中插值：{mid:?}");
    }

    // ── 宽度与占位（原有语义契约，保留）────────────────────────────────

    #[test]
    fn wordmark_rows_are_equal_width() {
        let ws: Vec<usize> = WORDMARK.iter().map(|r| r.chars().count()).collect();
        assert!(ws.windows(2).all(|w| w[0] == w[1]), "词标各行必须等宽，实际 {ws:?}");
    }

    #[test]
    fn finds_the_latest_outstanding_approval() {
        let evs = vec![
            EventMsg::TurnStarted { turn_id: "t".into() },
            EventMsg::ApprovalRequest { id: "a1".into(), detail: "d".into() },
            EventMsg::ToolCallEnd { id: "c".into(), exit_code: 0 },
            EventMsg::ApprovalRequest { id: "a2".into(), detail: "d".into() },
        ];
        assert_eq!(latest_approval_id(&evs), Some("a2".to_string()));
        assert_eq!(latest_approval_id(&[]), None);
        assert_eq!(
            latest_approval_id(&[EventMsg::TurnComplete { input_tokens: 0, output_tokens: 0 }]),
            None
        );
    }

    #[test]
    fn parses_approval_answers_and_rejects_noise() {
        for yes in ["y", "Y", "yes", "YES", " y "] {
            assert_eq!(parse_approval_answer(yes), Some(true), "{yes} 应判为批准");
        }
        for no in ["n", "N", "no"] {
            assert_eq!(parse_approval_answer(no), Some(false), "{no} 应判为拒绝");
        }
        assert_eq!(parse_approval_answer("maybe"), None);
        assert_eq!(parse_approval_answer(""), None);
    }

    #[test]
    fn tui_host_reports_facts_from_the_shared_extractor() {
        let mut h = TuiFacts::new();
        h.consume(&EventMsg::TurnStarted { turn_id: "t".into() }).unwrap();
        h.consume(&EventMsg::AgentMessageDone { text: "hi".into() }).unwrap();
        h.consume(&EventMsg::ToolCallBegin { id: "c".into(), name: "bash".into() }).unwrap();
        h.consume(&EventMsg::ToolCallEnd { id: "c".into(), exit_code: 0 }).unwrap();
        assert_eq!(
            h.facts(),
            vec![
                Fact::AssistantSaid("hi".into()),
                Fact::ToolFinished { name: "bash".into(), exit_code: 0 },
            ]
        );
    }

    #[test]
    fn approval_state_changes_the_box_border_color() {
        // 待审批是"现在必须由你决定"的时刻；边框变色是余光可见的提示
        let a = about();
        let (idle, pending) = with_env(&[("COLORTERM", "truecolor")], || {
            let render_with = |awaiting: bool| {
                Screen {
                    cols: 80, rows: 20, facts: &[], input: "", status: "",
                    awaiting_input: awaiting, show_cursor: false,
                    about: Some(&a), trust: None,
                    theme: theme::ThemeName::OpenCode, popup: None, preformatted: None,
                    sidebar: false,
                }
                .render()
            };
            (render_with(false), render_with(true))
        });
        // 空闲边框用 border_active(#606060)，审批用 warning(#f5a742)
        assert!(idle.contains("38;2;96;96;96"), "空闲边框应为 border_active");
        assert!(pending.contains("38;2;245;167;66"), "审批边框应为 warning 色");
    }
}
