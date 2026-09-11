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

pub mod input;
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
        // 转义序列：再读最多 2 字节
        let mut seq = vec![0x1b];
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
/// 一屏内容，渲染成 ANSI 文本。
pub struct Screen<'a> {
    pub cols: usize,
    pub rows: usize,
    /// 已发生的用户可见事实（协议层 Fact，非宿主自造）
    pub facts: &'a [Fact],
    /// 当前输入行（纯文本；左侧竖条由渲染加，便于单独着色）
    pub input: &'a str,
    /// 状态栏（左侧）
    pub status: &'a str,
    /// 状态栏右侧信息（模型/档位等）；窄终端会自动让位
    pub footer_right: &'a str,
    /// 有未决审批：竖条转警告色，提示"现在该你回答"
    pub awaiting_input: bool,
    /// 光标可视（运行中不显示输入光标）
    pub show_cursor: bool,
    /// 首屏关于信息；仅在**尚无任何事实**时展示（有对话后让位给正文）
    pub about: Option<&'a About>,
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
    /// (R,G,B) —— 与 opencode 默认暗色主题同源
    fn rgb(self) -> (u8, u8, u8) {
        match self {
            Color::Primary => (0xfa, 0xb2, 0x83),
            Color::Accent => (0x9d, 0x7c, 0xd8),
            Color::Success => (0x7f, 0xd8, 0x8f),
            Color::Error => (0xe0, 0x6c, 0x75),
            Color::Warning => (0xf5, 0xa7, 0x42),
            Color::Info => (0x56, 0xb6, 0xc2),
            Color::Text => (0xee, 0xee, 0xee),
            Color::Muted => (0x80, 0x80, 0x80),
            Color::Border => (0x48, 0x48, 0x48),
            Color::BorderActive => (0x60, 0x60, 0x60),
        }
    }

    /// 256 色近似（16 色无法表达时用；数值取 xterm 256 色板最接近项）
    fn ansi256(self) -> u8 {
        match self {
            Color::Primary => 216, // #ffafaf
            Color::Accent => 140,  // #af87d7
            Color::Success => 114, // #87d787
            Color::Error => 168,   // #d75f87
            Color::Warning => 215, // #ffaf5f
            Color::Info => 73,     // #5fafaf
            Color::Text => 255,    // #eeeeee
            Color::Muted => 244,   // #808080
            Color::Border => 238,  // #444444
            Color::BorderActive => 241, // #626262
        }
    }

    /// 16 色兜底（老终端）
    fn ansi16(self) -> u8 {
        match self {
            Color::Primary | Color::Warning => 33,
            Color::Accent => 35,
            Color::Success => 32,
            Color::Error => 31,
            Color::Info => 36,
            Color::Text => 37,
            Color::Muted | Color::Border | Color::BorderActive => 90,
        }
    }

    fn fg(self, mode: ColorMode) -> String {
        let (r, g, b) = self.rgb();
        match mode {
            ColorMode::None => String::new(),
            ColorMode::TrueColor => format!("{ESC}[38;2;{r};{g};{b}m"),
            ColorMode::Ansi256 => format!("{ESC}[38;5;{}m", self.ansi256()),
            ColorMode::Ansi16 => format!("{ESC}[{}m", self.ansi16()),
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
const BOLD: &str = "\u{1b}[1m";


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
}

impl Pal {
    fn new(mode: ColorMode) -> Self {
        Self {
            primary: Color::Primary.fg(mode),
            accent: Color::Accent.fg(mode),
            success: Color::Success.fg(mode),
            error: Color::Error.fg(mode),
            warning: Color::Warning.fg(mode),
            info: Color::Info.fg(mode),
            text: Color::Text.fg(mode),
            muted: Color::Muted.fg(mode),
            border: Color::Border.fg(mode),
            border_active: Color::BorderActive.fg(mode),
            dim: faint(mode),
            reset: if mode == ColorMode::None { String::new() } else { RESET.to_string() },
        }
    }
}

impl Screen<'_> {
    pub fn render(&self) -> String {
        // 每帧探测一次（两次环境变量读取，代价可忽略）；同时让测试能通过
        // 显式设置 NO_COLOR 来断言"无色"行为。
        let p = Pal::new(detect_color_mode());
        let rst = &p.reset;

        let mut out = String::new();
        // 移到左上并清屏（比逐行清除简单且无残留）
        out.push_str(&format!("{ESC}[H{ESC}[2J"));

        let input_rows = 3; // 输入行 + 状态栏 + 分隔
        let transcript_rows = self.rows.saturating_sub(input_rows);

        // 空对话时展示首屏；一旦有事实（含审批请求）就让位给正文。
        // 这比"启动时打印一次 banner 再清屏"更稳：不会在滚屏时留下残影。
        let lines = if self.facts.is_empty() {
            match self.about {
                Some(a) => self.welcome_lines(a, &p),
                None => Vec::new(),
            }
        } else {
            self.wrap_facts(self.cols, &p)
        };
        // 只显示最后 transcript_rows 行（自动滚到底）
        let start = lines.len().saturating_sub(transcript_rows);
        for line in &lines[start..] {
            out.push_str(line);
            out.push_str("\r\n");
        }
        // 补齐剩余空行，避免上一次的内容残留
        for _ in lines.len().saturating_sub(start)..transcript_rows {
            out.push_str("\r\n");
        }

        // 输入区：左侧竖条（对标 opencode 的 prompt 左边框 ┃），
        // 有未决审批时竖条转为警告色，让"现在该你回答"在余光里也看得到。
        // 空闲用 border_active、审批用 warning —— 与"用户消息"的 accent 竖条区分开，
        // 否则输入框和用户气泡会是同一种颜色，视觉上分不清"我在打字"与"我说过了"
        let bar = if self.awaiting_input { &p.warning } else { &p.border_active };
        out.push_str(&format!(
            "{}{}\u{2503}{}{} {}\r\n",
            bar, "", rst, p.text, self.input
        ));
        // 状态栏：左侧状态文本，右侧模型（对标 opencode footer 的左右分栏）
        let mut right = String::new();
        if !self.footer_right.is_empty() {
            right = format!("{}{}{}", p.muted, self.footer_right, rst);
        }
        out.push_str(&self.status_line(&p, &right));

        out
    }

    /// 状态栏：左状态、右信息，按**显示宽度**左右对齐（中文占 2 列）。
    fn status_line(&self, p: &Pal, right: &str) -> String {
        let rst = &p.reset;
        let left_plain = width::truncate_to_width(self.status, self.cols).to_string();
        let right_plain = strip_ansi(right);
        let lw = width::display_width(&left_plain);
        let rw = width::display_width(&right_plain);
        // 右侧信息在窄终端里会让位（宁可少显示，也不折行打乱布局）
        if lw + rw + 2 > self.cols || rw == 0 {
            return format!("{}{}{}{}", p.dim, p.muted, left_plain, rst);
        }
        let gap = self.cols - lw - rw;
        format!(
            "{}{}{}{}{}{}{}{}",
            p.dim, p.muted, left_plain, rst, " ".repeat(gap), p.muted, right_plain, rst
        )
    }

    /// 首屏内容：词标 + 会话信息 + 快捷键。
    ///
    /// **必须自己保证放得下**：`render` 只显示末尾 `transcript_rows` 行（自动滚到底），
    /// 若首屏比可视区高，被裁掉的恰好是**顶部**——用户会看到"没有 logo 的半截首屏"。
    /// 因此这里按「奢 → 简」四档试排，选第一个放得下的档位。
    fn welcome_lines(&self, a: &About, p: &Pal) -> Vec<String> {
        let avail = self.rows.saturating_sub(3); // 与 render 的 transcript_rows 同算式

        // (词标, 副标题, 快捷键, 留白)
        for &(mark, subtitle, hints, airy) in &[
            (true, true, true, true),
            (false, true, true, true),
            (false, true, true, false),
            (false, false, true, false),
        ] {
            let lines = self.welcome_variant(a, mark, subtitle, hints, airy, p);
            if lines.len() <= avail {
                return lines;
            }
        }
        // 极端小的终端：只留最要紧的一行 + 键值
        self.welcome_variant(a, false, false, false, false, p)
    }

    fn welcome_variant(
        &self,
        a: &About,
        mark: bool,
        subtitle: bool,
        hints: bool,
        airy: bool,
        p: &Pal,
    ) -> Vec<String> {
        let rst = &p.reset;
        // 定长文案也按宽度截断（窄终端里提示语会超宽）
        let fit = |s: &str| width::truncate_to_width(s, self.cols.saturating_sub(2)).to_string();
        // 键值先截断再着色：着色后含 ANSI，再按宽度截会错切
        let val_budget = self.cols.saturating_sub(16);
        let cut = |s: &str| width::truncate_to_width(s, val_budget).to_string();

        let mut lines: Vec<String> = Vec::new();
        if airy {
            lines.push(String::new());
        }

        if mark && self.cols >= WORDMARK_MIN_COLS {
            for row in WORDMARK {
                lines.push(format!("  {}{row}{rst}", p.primary));
            }
        } else {
            lines.push(format!("  {BOLD}{}NEO{rst}", p.primary));
        }

        lines.push(String::new());
        lines.push(format!(
            "  {BOLD}{}Neo{rst}{} —— 编程 Agent 内核{rst}",
            p.text, p.muted
        ));
        if subtitle {
            lines.push(format!(
                "  {}{}{rst}",
                p.info,
                fit("Rust 内核 · TUI / Web / Exec 共享同一内核")
            ));
        }

        if airy {
            lines.push(String::new());
        }
        for (label, value) in [
            ("版本", &a.version),
            ("模型", &a.model),
            ("模式", &a.mode),
            ("工作区", &a.workspace),
            ("会话", &a.session),
        ] {
            // pad_to_width 按**显示列**对齐（中文标签 1 字 = 2 列，不能按字符个数 pad）
            let padded = width::pad_to_width(label, 10);
            lines.push(format!("  {}{padded}{rst}{}", p.muted, cut(value)));
        }

        if hints {
            lines.push(String::new());
            lines.push(format!(
                "  {}{}{rst}",
                p.border,
                fit("输入任务后回车提交 · Ctrl+R 搜索历史 · Tab 补全 @文件引用")
            ));
            lines.push(format!(
                "  {}{}{rst}",
                p.border,
                fit("Ctrl+G 外部编辑器 · Ctrl+L 清屏 · Ctrl+C 退出")
            ));
        }
        lines
    }

    /// 把 Fact 列表渲染成若干行文本（已含 ANSI）。
    ///
    /// 视觉语言对标 opencode：
    ///   - 用户消息带左侧竖条（与助手正文区分开）
    ///   - 助手正文不加框，直接跟在后面（opencode 的 assistant 就是纯文本）
    ///   - 工具调用走"树状"缩进，成功 ✓ / 失败 ✗，细节（exit code）压暗
    ///   - 元信息（token、会话）一律 muted，不抢正文
    fn wrap_facts(&self, cols: usize, p: &Pal) -> Vec<String> {
        let rst = &p.reset;
        let inner = cols.saturating_sub(4);
        let mut lines = Vec::new();
        for f in self.facts {
            match f {
                Fact::UserSaid(text) => {
                    // 左侧竖条给用户消息一个"我说的话"的视觉归属
                    let mut first = true;
                    for l in text.lines() {
                        for w in width::wrap_to_width(l, inner) {
                            if first {
                                lines.push(format!("{}\u{2503}{rst} {}{w}{rst}", p.accent, p.text));
                                first = false;
                            } else {
                                lines.push(format!("{}\u{2503}{rst}  {w}", p.accent));
                            }
                        }
                    }
                    lines.push(String::new());
                }
                Fact::AssistantSaid(text) => {
                    for l in text.lines() {
                        for w in width::wrap_to_width(l, inner) {
                            lines.push(w);
                        }
                    }
                    lines.push(String::new());
                }
                Fact::ToolFinished { name, exit_code } => {
                    // 成功/失败共用一个"树杈"形状，只换图标与颜色，行宽稳定
                    let (icon, color) = if *exit_code == 0 {
                        ("✓", &p.success)
                    } else {
                        ("✗", &p.error)
                    };
                    lines.push(format!(
                        "  {color}{icon}{rst} {name}{} exit {exit_code}{rst}",
                        p.muted
                    ));
                }
                Fact::ApprovalNeeded { detail } => {
                    // △ 与 opencode footer 的权限提示同形
                    lines.push(format!("  {}△ 需要审批：{detail}{rst}", p.warning));
                }
                Fact::Failed(msg) => lines.push(format!("  {}✗ {msg}{rst}", p.error)),
                Fact::TurnFinished { input_tokens, output_tokens } => lines.push(format!(
                    "  {}· {input_tokens} in / {output_tokens} out{rst}",
                    p.muted
                )),
                Fact::SessionReady { session_id } => {
                    lines.push(format!("  {}· 会话 {session_id}{rst}", p.muted))
                }
            }
        }
        lines
    }
}

/// 去掉 ANSI 转义序列（用于量宽/右侧对齐）。
fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            while let Some(&n) = it.peek() {
                it.next();
                if n == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
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

/// 状态栏文案：审批挂起时明确告诉用户该敲什么，否则是常规就绪提示。
fn idle_or_approval(outstanding: &Option<String>) -> String {
    match outstanding {
        Some(_) => "待审批 · 输入 y 批准 / n 拒绝 后回车".to_string(),
        None => "就绪 · 输入任务后回车 · Ctrl+C 退出".to_string(),
    }
}

/// 运行 TUI，直到用户退出。
///
/// `submit` 由调用方注入（通常是 `kernel.submit`），这样本 crate
/// **不依赖 neo-orchestration / L3 之上的任何东西**，只依赖契据。
pub fn run<F>(about: About, mut submit: F) -> std::io::Result<()>
where
    F: FnMut(neo_protocol::Op) -> Result<Vec<EventMsg>, String>,
{
    let raw = RawMode::enter().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    // panic 时也还原终端，否则用户的终端会被留在原始模式
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = set_stty("sane");
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

    // footer 右侧：模型 + 档位短名（对标 opencode footer 的右侧状态区）
    let footer_right = if about.mode_short.is_empty() {
        about.model.clone()
    } else {
        format!("{} · {}", about.model, about.mode_short)
    };

    loop {
        let (cols, rows) = terminal_size();
        let facts = facts_of(&events);
        let screen = Screen {
            cols,
            rows,
            facts: &facts,
            input: &input,
            status: &status,
            footer_right: &footer_right,
            awaiting_input: outstanding.is_some(),
            show_cursor: true,
            about: Some(&about),
        };
        write!(stdout, "{}", screen.render())?;
        stdout.flush()?;

        let key = read_key(&mut stdin);
        match key {
            Key::Quit => break,
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
                        footer_right: &footer_right,
                        awaiting_input: false,
                        show_cursor: false,
                        // 这一帧是"运行中"过渡态：facts 通常已非空，不展示首屏
                        about: None,
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

    let _ = raw.restore();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_basic_keys() {
        assert_eq!(decode_key(&[0x03]), Key::Quit);
        assert_eq!(decode_key(b"\r"), Key::Enter);
        assert_eq!(decode_key(&[0x7f]), Key::Backspace);
        assert_eq!(decode_key(b"a"), Key::Char('a'));
        assert_eq!(decode_key(&[0x1b, b'[', b'A']), Key::Up);
        assert_eq!(decode_key(&[0x1b, b'[', b'B']), Key::Down);
        assert_eq!(decode_key(&[0x1b]), Key::Unknown, "单独 ESC 不应被当方向键");
    }

    #[test]
    fn decodes_multibyte_utf8_keys() {
        // 中文输入必须正确解码：3 字节 UTF-8
        assert_eq!(decode_key("你".as_bytes()), Key::Char('你'));
        assert_eq!(decode_key("好".as_bytes()), Key::Char('好'));
        // 4 字节（emoji）
        assert_eq!(decode_key("🚀".as_bytes()), Key::Char('🚀'));
        // 非法序列不得 panic，也不得产出错误字符
        assert_eq!(decode_key(&[0xe5, 0x86]), Key::Unknown, "不完整的 UTF-8 应为 Unknown");
        assert_eq!(decode_key(&[0xff, 0xfe]), Key::Unknown, "非法字节应为 Unknown");
    }

    #[test]
    fn screen_shows_last_lines_when_transcript_overflows() {
        // 事实多于可用行时，应显示**最新的**（自动滚到底），而不是截掉尾部
        let facts = vec![
            Fact::AssistantSaid("line1".into()),
            Fact::AssistantSaid("line2".into()),
            Fact::AssistantSaid("line3".into()),
        ];
        let s = Screen {
            cols: 40,
            rows: 6,
            facts: &facts,
            input: "",
            status: "s",
            footer_right: "",
            awaiting_input: false,
show_cursor: false,
            about: None,
        };
        let out = s.render();
        assert!(out.contains("line3"), "应显示最新内容：{out}");
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
        // 无法识别的不提交（避免误批）
        assert_eq!(parse_approval_answer("maybe"), None);
        assert_eq!(parse_approval_answer(""), None);
    }

    #[test]
    fn transcript_shows_the_user_message_with_its_own_bar() {
        // 转录必须能看出"我问了什么"，且与助手正文在视觉上可区分
        let facts = vec![
            Fact::UserSaid("列一下文件".into()),
            Fact::AssistantSaid("好的".into()),
        ];
        let s = Screen {
            cols: 60,
            rows: 20,
            facts: &facts,
            input: "",
            status: "就绪",
            footer_right: "",
            awaiting_input: false,
            show_cursor: true,
            about: None,
        };
        let out = s.render();
        assert!(out.contains("列一下文件"), "用户消息必须可见：{out}");
        assert!(out.contains('\u{2503}'), "用户消息应带左侧竖条：{out}");
        assert!(out.contains("好的"), "助手正文必须可见：{out}");
    }

    #[test]
    fn tool_success_and_failure_are_visually_distinguishable() {
        let ok = vec![Fact::ToolFinished { name: "bash".into(), exit_code: 0 }];
        let bad = vec![Fact::ToolFinished { name: "bash".into(), exit_code: 1 }];
        // 用嵌套 fn 而非闭包：Screen<'a> 借入 facts，闭包推断的生命周期不够长
        fn render_facts(f: &[Fact]) -> String {
            Screen {
                cols: 60, rows: 10, facts: f, input: "", status: "s",
                footer_right: "", awaiting_input: false, show_cursor: false, about: None,
            }
            .render()
        }
        let a = render_facts(&ok);
        let b = render_facts(&bad);
        assert!(a.contains('✓'), "成功应显示 ✓：{a}");
        assert!(b.contains('✗'), "失败应显示 ✗：{b}");
    }

    #[test]
    fn no_color_env_disables_all_coloring() {
        // NO_COLOR 是硬约定：设了就不能上色（无障碍 / 重定向到文件的场景）
        std::env::set_var("NO_COLOR", "1");
        let facts = vec![
            Fact::UserSaid("hi".into()),
            Fact::AssistantSaid("yo".into()),
            Fact::ToolFinished { name: "bash".into(), exit_code: 0 },
        ];
        let s = Screen {
            cols: 60, rows: 20, facts: &facts, input: "x", status: "s",
            footer_right: "m", awaiting_input: false, show_cursor: false, about: None,
        };
        let out = s.render();
        std::env::remove_var("NO_COLOR");
        // 只允许定位/光标类转义（如 [H、[2J），不允许任何颜色 SGR（以 m 结尾）
        let colored: Vec<&str> = out.split('\u{1b}')
            .filter(|seg| seg.contains('m') && seg.chars().next().map(|c| c.is_ascii_digit() || c == '[').unwrap_or(false))
            .filter(|seg| {
                let head: String = seg.chars().take_while(|c| *c != 'm').collect();
                head.chars().all(|c| c.is_ascii_digit() || c == ';' || c == '[')
            })
            .collect();
        assert!(colored.is_empty(), "NO_COLOR 下不应有颜色转义：{colored:?}");
    }

    #[test]
    fn footer_right_yields_on_narrow_terminals() {
        // 窄终端里右侧信息让位，而不是折行打乱布局
        let facts = vec![Fact::AssistantSaid("x".into())];
        let s = Screen {
            cols: 24, rows: 10, facts: &facts, input: "", status: "就绪 · 很长很长很长",
            footer_right: "deepseek-chat · auto-edit", awaiting_input: false,
            show_cursor: false, about: None,
        };
        let out = s.render();
        assert!(out.contains("就绪"), "左侧状态应保留：{out}");
        assert!(!out.contains("deepseek-chat"), "窄终端应舍弃右侧信息：{out}");
    }

    #[test]
    fn status_line_aligns_right_info_to_the_edge() {
        let s = Screen {
            cols: 40, rows: 10, facts: &[], input: "", status: "就绪",
            footer_right: "mock", awaiting_input: false, show_cursor: false, about: None,
        };
        let line = s.status_line(&Pal::new(ColorMode::None), "mock");
        assert_eq!(width::display_width(&line), 40, "状态栏应铺满整行：{line:?}");
        assert!(line.starts_with("就绪"), "{line:?}");
        assert!(line.ends_with("mock"), "{line:?}");
    }

    #[test]
    fn wordmark_rows_are_equal_width() {
        // 手改词标最容易犯的错：某行多/少一个字符，终端里立刻看出错位。
        let ws: Vec<usize> = WORDMARK.iter().map(|r| r.chars().count()).collect();
        assert!(
            ws.windows(2).all(|w| w[0] == w[1]),
            "词标各行必须等宽，实际 {ws:?}"
        );
    }

    fn about_fixture() -> About {
        About {
            version: "0.1.0".into(),
            model: "mock".into(),
            mode: "Default（沙箱 WorkspaceWrite / 审批 OnRequest）".into(),
            mode_short: "default".into(),
            workspace: "/tmp/ws".into(),
            session: "neo-tui".into(),
        }
    }

    #[test]
    fn empty_transcript_shows_the_welcome_screen() {
        let a = about_fixture();
        let s = Screen {
            cols: 100,
            rows: 30,
            facts: &[],
            input: "",
            status: "就绪",
            footer_right: "",
            awaiting_input: false,
show_cursor: true,
            about: Some(&a),
        };
        let out = s.render();
        // 注入的会话信息必须出现（否则首屏等于没有信息量）
        assert!(out.contains("mock"), "首屏应显示模型：{out}");
        assert!(out.contains("/tmp/ws"), "首屏应显示工作区：{out}");
        assert!(out.contains("neo-tui"), "首屏应显示会话：{out}");
        assert!(out.contains("0.1.0"), "首屏应显示版本：{out}");
        // 大终端下应出现词标
        assert!(out.contains('█'), "大终端应显示词标：{out}");
    }

    #[test]
    fn welcome_yields_to_content_once_there_are_facts() {
        // 有内容后首屏必须消失，否则每轮都刷一遍 banner 会淹没正文
        let a = about_fixture();
        let facts = vec![Fact::AssistantSaid("回答".into())];
        let s = Screen {
            cols: 100,
            rows: 30,
            facts: &facts,
            input: "",
            status: "就绪",
            footer_right: "",
            awaiting_input: false,
show_cursor: true,
            about: Some(&a),
        };
        let out = s.render();
        assert!(out.contains("回答"), "正文必须显示：{out}");
        assert!(!out.contains('█'), "有正文时不该再出现词标：{out}");
    }

    #[test]
    fn small_terminal_degrades_to_one_line_wordmark() {
        // 小窗口里 6 行词标会把信息挤出可视区，应降级为单行
        let a = about_fixture();
        let s = Screen {
            cols: 40,
            rows: 12,
            facts: &[],
            input: "",
            status: "就绪",
            footer_right: "",
            awaiting_input: false,
show_cursor: true,
            about: Some(&a),
        };
        let out = s.render();
        assert!(out.contains("NEO"), "小终端应退化为单行 NEO：{out}");
        assert!(!out.contains('█'), "小终端不该显示大词标：{out}");
        assert!(out.contains("/tmp/ws"), "降级后信息仍须可见：{out}");
    }

    #[test]
    fn welcome_truncates_long_values_instead_of_overflowing() {
        // 长路径不能撑破行宽（宽字符截断用 display width，不用字节数）
        let a = About { workspace: "很长的目录名".repeat(30), ..about_fixture() };
        let s = Screen {
            cols: 40,
            rows: 30,
            facts: &[],
            input: "",
            status: "就绪",
            footer_right: "",
            awaiting_input: false,
show_cursor: true,
            about: Some(&a),
        };
        for line in s.welcome_lines(&a, &Pal::new(ColorMode::None)) {
            // 去掉 ANSI 后逐行量宽
            let plain: String = {
                let mut o = String::new();
                let mut it = line.chars().peekable();
                while let Some(c) = it.next() {
                    if c == '\u{1b}' {
                        while let Some(&n) = it.peek() {
                            it.next();
                            if n == 'm' { break; }
                        }
                    } else {
                        o.push(c);
                    }
                }
                o
            };
            assert!(
                width::display_width(&plain) <= s.cols,
                "行宽 {} 超过终端 {}：{plain:?}",
                width::display_width(&plain),
                s.cols
            );
        }
    }

    #[test]
    fn tui_host_reports_facts_from_the_shared_extractor() {
        // TUI 与 exec 必须用同一套事实语义（T6 的前提）
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
}
