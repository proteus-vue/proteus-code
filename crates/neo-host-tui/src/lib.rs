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

pub mod appearance;
pub mod commands;
pub mod diffview;
pub mod editor;
pub mod input;
pub mod popup;
pub mod markdown;
pub mod mouse;
pub mod stars;
pub mod view;
pub mod whichkey;
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

/// 鼠标上报模式（RAII）。
///
/// 开启 SGR 扩展模式（`?1006h`）+ 按键事件上报（`?1002h`：按下/释放与拖拽）。
/// 不用 `?1000h`（只报按下）是因为滚轮与拖拽在 1002 下才稳定。
///
/// **必须在退出时关闭**：否则 shell 会收到我们留下的鼠标序列，
/// 表现为"终端里鼠标乱跳、选中文本失灵"——这比 TUI 本身出错更难排查，
/// 因为用户已经退出到 shell 了。
struct MouseMode {
    active: bool,
}

impl MouseMode {
    fn enter(enabled: bool) -> Self {
        if !enabled {
            return Self { active: false };
        }
        let mut out = std::io::stdout();
        let _ = write!(out, "{ESC}[?1002h{ESC}[?1006h");
        let _ = out.flush();
        Self { active: true }
    }

    fn leave(&mut self) {
        if !self.active {
            return;
        }
        self.active = false;
        Self::leave_now();
    }

    /// 无需实例即可执行（panic 钩子用）。
    fn leave_now() {
        let mut out = std::io::stdout();
        let _ = write!(out, "{ESC}[?1006l{ESC}[?1002l");
        let _ = out.flush();
    }
}

impl Drop for MouseMode {
    fn drop(&mut self) {
        self.leave();
    }
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
        // 顺序：恢复光标 → 清屏 → 退出备用屏。
        //
        // - `?25h` 必须在 `?1049l` 之前：反了光标会在原屏幕上保持隐藏，
        //   用户回到 shell 发现看不到光标。
        // - `2J` 是给"把备用屏内容物化进 scrollback"的终端兜底（部分终端
        //   模拟器/内嵌终端在切换缓冲时不是丢弃而是保留）。有它的话，
        //   最坏情况也只是一块空白，而不是半截界面残留在提示符上方。
        let _ = write!(out, "{ESC}[?25h{ESC}[2J{ESC}[H{ESC}[?1049l");
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
    /// Ctrl+U 删到行首
    ClearLine,
    /// Ctrl+K 删到行尾
    DeleteToLineEnd,
    /// Ctrl+W 删前一个词
    DeleteWordBackward,
    /// Ctrl+Z 撤销
    Undo,
    /// Ctrl+Y 重做（readline 惯例）
    Redo,
    /// Delete 键（前向删除）
    Delete,
    /// Home / End
    Home,
    End,
    /// Alt+B / Alt+F 按词移动
    WordBackward,
    WordForward,
    /// PageUp / PageDown：整页滚动转录
    PageUp,
    PageDown,
    /// Ctrl+E / Ctrl+Y? 不 —— 用 Ctrl+E 到底、Ctrl+Home/End 跳首尾
    ScrollToBottom,
    ScrollToTop,
    /// Ctrl+F 打开搜索；Ctrl+N / Ctrl+P 下一个 / 上一个命中
    Search,
    SearchNext,
    SearchPrev,
    /// Ctrl+O：显示当前上下文的可用键（which-key）
    WhichKey,
    /// Ctrl+Z：挂起回 shell（前台交给用户）
    Suspend,
    /// 鼠标事件（SGR 扩展模式）
    Mouse(mouse::MouseEvent),
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
        [0x1b, b'[', b'5', b'~'] => Key::PageUp,
        [0x1b, b'[', b'6', b'~'] => Key::PageDown,
        [0x05] => Key::ScrollToBottom,
        [0x06] => Key::Search,
        [0x0f] => Key::WhichKey,
        [0x1f] => Key::Undo,
        [0x0e] => Key::SearchNext,
        [0x0b] => Key::DeleteToLineEnd,
        [0x17] => Key::DeleteWordBackward,
        // ── 控制字节分配（三者曾互相冲突，这里一次说清）──
        //   0x1a (ctrl+z) → 挂起回 shell（终端惯例，肌肉记忆最强）
        //   0x1f (ctrl+_) → 撤销（readline/shell 惯例）
        //   0x19 (ctrl+y) → 重做
        //   0x0f (ctrl+o) → which-key（ctrl+/ 也常发 0x1f，故不用它）
        // 冲突点：ctrl+z 原本是撤销、ctrl+/ 原本是 which-key，
        // 两者都想用 0x1a/0x1f。这里按"惯例强度"重排：挂起让给 ctrl+z、
        // 撤销让给 ctrl+_、which-key 退到 ctrl+o。
        [0x1a] => Key::Suspend,
        [0x19] => Key::Redo,
        [0x1b, b'[', b'3', b'~'] => Key::Delete,
        [0x1b, b'[', b'H'] | [0x1b, b'[', b'1', b'~'] => Key::Home,
        [0x1b, b'[', b'F'] | [0x1b, b'[', b'4', b'~'] => Key::End,
        [0x1b, b'b'] => Key::WordBackward,
        [0x1b, b'f'] => Key::WordForward,
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

/// 读一次按键；**最多等 `timeout_tenths` 个 0.1 秒**，无输入返回 `None`。
///
/// # 为什么需要超时
///
/// 阻塞读会把主循环钉死在 `read` 上，于是**窗口尺寸变化时无法重绘** ——
/// 终端只在输入到达时才被唤醒，而 resize 不产生输入。真机上表现为
/// "改窗口大小后画面错位、侧栏残留、大片空白"（用户实测截图）。
///
/// 这里用 `stty min 0 time N` 让 read 到点即返回。返回 0 字节表示**超时**
/// 而不是 EOF：我们会据此轮询尺寸，尺寸变了就重绘。
fn read_key_timeout(stdin: &mut impl Read, timeout_tenths: u8) -> Option<Key> {
    let _ = set_stty(&format!("min 0 time {timeout_tenths}"));
    let mut b = [0u8; 1];
    if stdin.read(&mut b).unwrap_or(0) == 0 {
        // 超时（无输入）。真 EOF 在交互场景不会出现：退出走 Ctrl+C / Ctrl+D，
        // 它们会送来 0x03 / 0x04 字节而不是 EOF。
        return None;
    }
    // **不要**在这里重新 `stty raw`。
    //
    // `raw` 会把 `min`/`time` 复位成**阻塞**（min=1），于是下面
    // `decode_first` 为转义序列续读的那几字节会永久阻塞：
    //   · 方向键没事 —— `ESC [ A` 三个字节早已在缓冲里，续读立即命中；
    //   · 单独按 ESC **会卡死** —— 后面没有字节，read 一直等，
    //     界面因此"按了没反应"，而下一个按键还会被当成序列的续字节吃掉。
    // 这正是用户报的"q 能退出、esc 不行"。
    // 保持 `min 0 time N`，单独 ESC 会在 N 个 0.1 秒后落到 `[0x1b] => Escape`。
    Some(decode_first(&b, stdin))
}

/// 从已读到的首字节 + stdin 续读，解出一个按键。
fn decode_first(first: &[u8; 1], stdin: &mut impl Read) -> Key {
    let b = first[0];
    if b == 0x1b {
        // 转义序列：最多再读 3 字节（总计 4）。
        //
        // 需要 4 字节是因为 `ESC [ 3 ~`（Delete）、`ESC [ 1 ~`（Home）
        // 这类序列有 4 个字节 —— 之前只读 2 个额外字节，它们会被截断成
        // 前缀而永远匹配不上。超时设置已由调用方就位，所以单独按 Esc
        // （后面没有字节）仍会立刻返回，不会卡住。
        let mut seq = vec![0x1b];
        for _ in 0..3 {
            let mut c = [0u8; 1];
            if stdin.read(&mut c).unwrap_or(0) == 0 {
                break;
            }
            seq.push(c[0]);
            // 终止字节：`~` 或字母（A-Za-z）说明序列已完整
            if c[0] == b'~' || c[0].is_ascii_alphabetic() {
                break;
            }
        }

        // 鼠标（SGR）：`ESC [ < b;x;y M|m` —— 比普通转义序列长得多
        // （宽终端下可达十几个字节），所以单独一路读到终止符。
        // 必须**有上限**：终端若送来畸形的半截序列，无上限读会把循环卡死。
        // 注意：上面的循环已经读走了最多 3 个字节，所以这里可能已经是 4 字节
        // （`ESC [ <` 加上第一个参数字节）。用 `>=` 而不是 `==` ——
        // 写成 `== 3` 会让判断永远不成立，鼠标彻底失效。
        if seq.len() >= 3 && seq[1] == b'[' && seq[2] == b'<' {
            const MAX_MOUSE_BYTES: usize = 46; // 足够容纳 5 位数的宽坐标
            while seq.len() < MAX_MOUSE_BYTES {
                let mut c = [0u8; 1];
                if stdin.read(&mut c).unwrap_or(0) == 0 {
                    break;
                }
                seq.push(c[0]);
                if c[0] == b'M' || c[0] == b'm' {
                    break;
                }
            }
            let body = String::from_utf8_lossy(&seq[3..]);
            return match mouse::parse_sgr(&body) {
                Some(ev) => Key::Mouse(ev),
                None => Key::Unknown,
            };
        }
        return decode_key(&seq);
    }
    if b < 0x80 {
        return decode_key(&[b]);
    }
    // 多字节 UTF-8：按首字节判断续字节数
    let need = match b {
        0xc0..=0xdf => 1,
        0xe0..=0xef => 2,
        0xf0..=0xf7 => 3,
        _ => 0,
    };
    let mut buf = vec![b];
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
    /// 可选模型清单：(名字, 描述, 是否可用于真实任务)。
    ///
    /// **由调用方注入**：宿主不持有内核，也不知道注册表长什么样。
    /// 这样"模型列表"是数据，宿主只负责显示与把选择转成 Op。
    pub models: Vec<(String, String, bool)>,
    /// 当前模型名（可与 `model` 不同：切换后要显示新的）
    pub current_model: String,
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
    /// 背景色的 SGR。主题里三档层次必须**可区分但都不抢戏**：
    /// 面板最接近底色，表面稍亮（卡片要"浮起来"），选中是低饱和强调色。
    fn bg(&self, b: Bg) -> String {
        let t = &self.theme;
        let (r, g, bl) = match b {
            Bg::Panel => t.bg_panel,
            Bg::Surface => t.bg_surface,
            Bg::Selected => t.bg_selected,
        };
        Color::Rgb(r, g, bl).bg(self.mode, t)
    }

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
    /// 当前输入（多行编辑器；左边框由渲染加，便于单独着色）
    pub input: &'a editor::Editor,
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
    /// 转录滚动位置与搜索（`None` = 新建默认视图，贴底跟随）
    pub view: Option<&'a view::View>,
    /// 全屏 diff 查看器（`Some` = 占满屏幕）
    pub diff_viewer: Option<&'a diffview::Viewer>,
    /// which-key 提示分组（`Some` = 覆盖层，任意键关闭）
    pub whichkey: Option<&'a [whichkey::Group]>,
    /// 工具输出与推理的显示方式
    pub display: ToolDisplay,
    /// 设置视图（`Some` = 占满屏幕）
    pub settings: Option<&'a Vec<SettingSection>>,
    /// 设置视图当前选中的可操作行（用于高亮）
    pub settings_cursor: usize,
    /// 设置页内的**内联选择器**（在该行下方就地展开候选）。
    ///
    /// 为什么不用居中弹窗：设置页是全屏的，一点回车却退回聊天画面弹个小框，
    /// 观感是"两个界面来回跳"。内联展开让候选长在那一行下面 ——
    /// 全程留在同一页里，移动和选择都在原上下文完成。
    pub settings_picker: Option<&'a SettingsPicker>,
    /// 审批对话框（模态卡片）。`Some` = 显示并接管输入。
    pub approval: Option<&'a ApprovalPrompt>,
    /// 设置页里的表单（新增/编辑服务商）。`Some` = 显示表单并接管输入。
    pub settings_form: Option<&'a SettingsForm>,
    /// 危险操作的确认提示（如删除服务商）。`Some` = 显示并等待 y/n。
    pub settings_confirm: Option<&'a str>,
    /// 外观（背景纹理 + Logo 样式）
    pub appearance: appearance::Appearance,
    /// 自定义背景字符画（`NEO_TUI_BG_FILE` 读入；优先于内置纹理）
    pub custom_background: Option<&'a Vec<String>>,
}

/// 审批对话框：把"需要你决定"做成**模态卡片**，而不是在输入框里打字。
///
/// # 为什么不用"输入框里回 y/n"
///
/// 之前审批是"输入框变黄 + 提示 y/n"，用户要在对话框外打字 —— 观感与
/// opencode/mimo 那种"弹出确认框、↑↓ 选、回车确认"差很远（用户明确反馈）。
/// 而且 `y` 需要敲进输入框再回车，不是一次按键。
///
/// 现在做成居中卡片：显示"要做什么"（工具与参数）+ 改动预览 + 三个选项，
/// ↑↓ 选择 / 回车确认，y/n/a 作为快捷键直接生效。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalPrompt {
    /// 这次要审批的说明（内核给的 detail）
    pub detail: String,
    /// 要执行的调用摘要（工具名 + 关键参数），让用户知道"批的是什么"
    pub call: String,
    /// 改动预览（路径, unified diff）—— 有则显示
    pub diff: Option<(String, String)>,
    /// 0 = 批准一次 · 1 = 总是允许这类 · 2 = 拒绝
    pub selected: usize,
}

impl ApprovalPrompt {
    /// 三个选项的文案（`selected` 是下标）。
    pub const CHOICES: [&'static str; 3] =
        ["批准一次（y）", "总是允许这类（a）", "拒绝（n）"];

    pub fn choice_at(&self) -> neo_protocol::Decision {
        match self.selected {
            0 => neo_protocol::Decision::Allow,
            1 => neo_protocol::Decision::AllowAlways,
            _ => neo_protocol::Decision::Deny,
        }
    }
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

    /// 背景色 SGR。与 `fg` 对称，只改参数项（48;2 / 48;5 / 40–47）。
    /// `Ansi16` 下退到暗色系（40–47）—— 16 色档没有精确的"浅灰面板"，
    /// 但深色背景块足以表达层级（比不画好）。
    fn bg(self, mode: ColorMode, t: &theme::Theme) -> String {
        let (r, g, b) = self.rgb(t);
        match mode {
            ColorMode::None => String::new(),
            ColorMode::TrueColor => format!("{ESC}[48;2;{r};{g};{b}m"),
            ColorMode::Ansi256 => format!("{ESC}[48;5;{}m", rgb_to_256(r, g, b)),
            ColorMode::Ansi16 => format!("{ESC}[40m"),
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

/// 输入区最多显示多少行（超出滚动到末尾）。
/// 上限是必要的：输入框不能吃掉整个屏幕 —— 正文才是主体。
const MAX_INPUT_ROWS: usize = 6;

/// 底部固定区块行数（单行输入时）：输入框(3) + 提示行(1) + 状态行(1) + 边框(1)
const CHROME_ROWS: usize = 6;

/// 按输入行数算出 chrome 实际占用行数。
///
/// 输入框高度是**动态**的（单行 → 多行），因此不能用常量代替。
/// 布局里凡是"正文可用行数"的地方都必须走这个函数。
fn chrome_rows(input_rows: usize) -> usize {
    let shown = input_rows.clamp(1, MAX_INPUT_ROWS);
    // 上边框 + 输入行(shown) + 状态行 + 下边框 + 提示行 + 底部状态行
    2 + shown + 1 + 1 + 1
}

/// 侧栏宽度（对齐 opencode 的 42 列；我们窄一些，因为终端普遍没它宽）。
const SIDEBAR_COLS: usize = 34;
/// 显示侧栏所需的最小终端宽度。低于此值强制隐藏 ——
/// 正文被挤到 40 列以下时，侧栏带来的信息量抵不上阅读体验的损失。
const SIDEBAR_MIN_COLS: usize = 96;

/// 一行的事实片段：(起始列, 文本, 色调)
pub type Seg = (usize, String, Tone);

/// 渲染时记录的命中区域。
///
/// 为什么由渲染产出而不是输入时另算一遍：**只有渲染知道东西画在哪**。
/// 另算一遍就等于把布局逻辑写两遍，两边一旦不一致，鼠标就会点错地方
/// （而且这种错很难归因）。渲染顺手记下来是唯一不会漂移的做法。
#[derive(Debug, Clone, Default)]
pub struct Regions {
    /// 转录正文区（用于滚轮）：(top, bottom_exclusive)
    pub transcript: Option<(usize, usize)>,
    /// 侧栏区域（用于点击）：(x0, x1_exclusive, y0, y1_exclusive)
    pub sidebar: Option<(usize, usize, usize, usize)>,
    /// 输入框区域：(left, right_exclusive, top, bottom_exclusive)
    pub input_box: Option<(usize, usize, usize, usize)>,
    /// 弹窗候选行：每项是 (y 行号, x0, x1) 与对应的选项下标
    pub popup_items: Vec<((usize, usize, usize), usize)>,
    /// 状态行上的"下方还有 N 行"提示（点击即到底）
    pub scroll_hint_rows: Vec<usize>,
    /// 侧栏收起时的"把手"格子（列, 行，1 基）—— 点它展开侧栏。
    /// 没有它的话，鼠标用户一旦点收起就再也点不开了（陷阱）。
    pub sidebar_grip: Option<(usize, usize)>,
    /// 设置页的可操作行：`(y 行号, 可操作序号)`。
    ///
    /// 只登记**可操作项**，与 `settings_cursor` 同一坐标空间 ——
    /// 点击后直接复用 Enter 的分支，两条输入路径不会各算一套。
    pub settings_rows: Vec<(usize, usize)>,
}
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

// 提示行必须能同时塞下左右两栏（否则右栏会被丢弃，提示就白写了）。
// 76 列的输入框里两栏合计要留得住空档，所以每栏只放最高频的几个键；
// 完整键位在 `/keys` 里。
// 提示行必须能同时塞下两栏（76 列框内可用约 68 列），否则右栏被丢弃。
// 完整键位在 `/keys`。
const HINT_LEFT: &str = "tab 补全  ctrl+/ 键位提示";
const HINT_RIGHT: &str = "@ 引用  pgup/pgdn 滚动  ctrl+c 退出";

/// 屏幕网格。
///
/// 为什么先铺格子再输出，而不是直接拼字符串：
/// 星场要能出现在**居中内容的左右两侧**。若按"内容 + 补星星"来做，
/// 左侧空白已算进内容宽度，星星永远进不去左边。格子模型把
/// "内容覆盖"与"背景填充"分开，两边都能正确落星。
/// 一块背景色。用主题里的**三个层次**表达"面"：
/// 面板（侧栏/设置页底）→ 表面（模态卡片）→ 选中（列表高亮条）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bg {
    Panel,
    Surface,
    Selected,
}

struct Grid {
    cols: usize,
    rows: usize,
    ch: Vec<char>,
    tone: Vec<Tone>,
    /// 每格的**背景**（`None` = 用终端默认底色）。
    ///
    /// 单独一层而不是塞进 `tone`：前景与背景是正交的两个概念
    /// （同一个面板底色上可以放 Text/Warning/Muted 各种前景）。
    /// 合进 tone 会让组合爆炸（每种前景 × 每种背景）。
    bg: Vec<Option<Bg>>,
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
            bg: vec![None; n],
            skip: vec![false; n],
            put_limit: usize::MAX,
        }
    }

    /// 限制写入范围的列上限（`usize::MAX` = 不限制）。侧栏存在时设为侧栏左边界，
    /// 避免正文写到侧栏下面再被覆盖（那会在视觉上"截断"正文，很难解释）。
    fn clamp_put(&mut self, until: usize) {
        self.put_limit = until;
    }

    /// 给一块矩形铺**底色**（行区间 `r0..r1`、列区间 `c0..c1`，不含右端）。
    ///
    /// 这是"层级感"的来源：终端里只有底色能表达"这块是一个独立的面"。
    /// 只设背景、不动字符 —— 所以可以在内容之前或之后调用
    /// （之后调用不会盖掉字符，只补底色）。
    fn fill_bg(&mut self, r0: usize, r1: usize, c0: usize, c1: usize, bg: Bg) {
        for r in r0..r1.min(self.rows) {
            for c in c0..c1.min(self.cols).min(self.put_limit) {
                let i = r * self.cols + c;
                self.bg[i] = Some(bg);
            }
        }
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

    /// 居中写一行（无色调片段时用）。
    fn put_centered(&mut self, row: usize, text: &str, tone: Tone) {
        let w = width::display_width(text);
        let col = self.cols.saturating_sub(w) / 2;
        self.put(row, col, text, tone);
    }

    /// 给尚未写入的格子铺背景纹理。**确定性**，所以多帧之间完全静止。
    ///
    /// 自定义字符画（`rows` 非空时）优先：它按行给出字符，
    /// 超出画布尺寸时按取模重复，形成平铺效果。
    fn fill_background(&mut self, bg: appearance::Background, custom: Option<&Vec<String>>) {
        if let Some(art) = custom {
            let ar = art.len();
            for r in 0..self.rows {
                let line = &art[r % ar];
                let chars: Vec<char> = line.chars().collect();
                if chars.is_empty() {
                    continue;
                }
                for c in 0..self.cols {
                    let i = r * self.cols + c;
                    if self.tone[i] != Tone::None {
                        continue;
                    }
                    let ch = chars[c % chars.len()];
                    if ch == ' ' {
                        continue; // 空格视为"透空"，保留纯色
                    }
                    self.ch[i] = ch;
                    self.tone[i] = Tone::Border;
                }
            }
            return;
        }
        for r in 0..self.rows {
            for c in 0..self.cols {
                let i = r * self.cols + c;
                if self.tone[i] != Tone::None {
                    continue;
                }
                if let Some((ch, bright)) = bg.cell(r, c, self.cols) {
                    self.ch[i] = ch;
                    self.tone[i] = match bright {
                        appearance::Brightness::Bright => Tone::StarBright,
                        appearance::Brightness::Dim => Tone::StarDim,
                    };
                }
            }
        }
    }

    /// 旧的星场入口（保留给测试与向后兼容）。
    #[allow(dead_code)]
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
                let mut cur_bg: Option<Bg> = None;
                let mut started = false;
                for c in 0..self.cols {
                    let i = r * self.cols + c;
                    if self.skip[i] {
                        continue;
                    }
                    let t = self.tone[i];
                    let b = self.bg[i];
                    // 前景或背景任一变化都要重发 SGR（并先 reset）
                    if !started || t != cur || b != cur_bg {
                        if started {
                            out.push_str(&p.reset);
                        }
                        if let Some(bg) = b {
                            out.push_str(&p.bg(bg));
                        }
                        out.push_str(&p.tone(t));
                        cur = t;
                        cur_bg = b;
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
    /// 兼容旧调用：只要渲染结果。
    pub fn render(&self) -> String {
        self.render_with_regions().0
    }

    pub fn render_with_regions(&self) -> (String, Regions) {
        let p = Pal::new(detect_color_mode(), self.theme);
        let mut g = Grid::new(self.cols, self.rows);
        let mut regions = Regions::default();

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

        // 全屏 diff 查看器：占满整屏，不画输入框/侧栏/状态栏。
        if let Some(v) = self.diff_viewer {
            self.draw_diff_viewer(&mut g, &p, v);
            g.fill_background(appearance::Background::None, None);
            let mut out = format!("{ESC}[H{ESC}[2J");
            out.push_str(&g.lines(&p).join("\r\n"));
            out.push_str(&format!("{ESC}[?25l"));
            return (out, regions);
        }

        // 设置视图：占满整屏
        if let Some(sections) = self.settings {
            // 设置页是"另一个页面"，整屏铺面板底色，与对话页明确分开
            g.fill_bg(0, self.rows, 0, self.cols.saturating_sub(1), Bg::Panel);
            // 先铺背景纹理，再画内容 —— **顺序是唯一正确的**。
            //
            // `fill_background` 只填 `Tone::None`（空）格、不覆盖已有内容，
            // 所以必须在内容之前调用：反过来的话整屏都已有内容，
            // 纹理会一个格子都画不出来（表现为"点了背景只换了名字、画面没变"）。
            // 主界面几处渲染都是这个顺序，设置页曾写反。
            g.fill_background(self.appearance.background, self.custom_background);
            let cur = self.draw_settings(&mut g, sections, &mut regions);
            let mut out = format!("{ESC}[H{ESC}[2J");
            out.push_str(&g.lines(&p).join("\r\n"));
            // 表单打开时要显示硬件光标（否则输入看不见插入点）
            match cur {
                Some((r, c)) => out.push_str(&format!("{ESC}[{r};{c}H{ESC}[?25h")),
                None => out.push_str(&format!("{ESC}[?25l")),
            }
            return (out, regions);
        }

        let chrome = chrome_rows(self.input.line_count());
        let body_rows = self.rows.saturating_sub(chrome);
        let (chrome_top, cursor) = if let Some(lines) = self.preformatted {
            // 信息屏是**文档**：必须从第一行开始显示。
            // 曾用"显示末尾 N 行"（对话滚屏的逻辑），结果长帮助把标题裁掉、
            // 只留中间 —— 文档不能倒着读。
            let avail = self.rows.saturating_sub(chrome);
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
            let top = self.rows.saturating_sub(chrome);
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
        // which-key 覆盖层：放在正文区右下角，不压输入框（用户还要继续打字）
        if let Some(groups) = self.whichkey {
            self.draw_whichkey(&mut g, groups, chrome_top);
        }
        // 审批对话框：**最上层模态**。它要求用户先回答才能继续，
        // 所以画在弹窗/which-key 之上（那些是"看一眼"的信息，不该盖住提问）。
        if let Some(ap) = self.approval {
            self.draw_approval(&mut g, ap, chrome_top);
        }

        // 侧栏最后画：它占的是自己的列区，且要求不被正文侵入
        if let Some(x0) = side_x0 {
            g.clamp_put(usize::MAX);
            self.draw_sidebar(&mut g, &p, x0);
            regions.sidebar = Some((x0, self.cols, 0, self.rows));
        }
        // 侧栏收起：在最右一格画可点击的把手，避免"收起后无法用鼠标展开"
        if side_x0.is_none() && self.cols > 8 && self.rows > 2 {
            let gy = self.rows - 1; // 底行
            let gx = self.cols - 1;
            g.put(gy, gx, "‹", Tone::BorderActive);
            regions.sidebar_grip = Some((gx + 1, gy + 1)); // 转 1 基
        }

        // 记录命中区域（鼠标用）。只在"对话/首页"这类有转录的界面记录；
        // 信息屏与信任页不参与鼠标交互。
        if self.trust.is_none() && self.preformatted.is_none() {
            regions.transcript = Some((0, body_rows));
        }
        let body = self.body_cols();
        let box_w = self.box_width();
        let left = body.saturating_sub(box_w) / 2;
        regions.input_box = Some((left, left + box_w, chrome_top, self.rows));
        if let Some(pop) = self.popup {
            let _ = pop;
            regions.popup_items = self.popup_item_rows(chrome_top);
        }

        g.fill_background(self.appearance.background, self.custom_background);
        let mut out = format!("{ESC}[H{ESC}[2J");
        out.push_str(&g.lines(&p).join("\r\n"));
        match cursor {
            // 用真实终端光标（而不是画一个 ▌），输入手感与原生一致
            Some((r, c)) => out.push_str(&format!("{ESC}[{r};{c}H{ESC}[?25h")),
            None => out.push_str(&format!("{ESC}[?25l")),
        }
        (out, regions)
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
        let body = self.rows.saturating_sub(chrome_rows(self.input.line_count()));
        let total = lines.len();

        // 滚动窗口由 View 决定；没有 View 时等价于"贴底跟随"
        let (start, end) = match self.view {
            Some(v) => v.window(total, body),
            None => (total.saturating_sub(body), total),
        };
        let shown = &lines[start..end.min(total)];
        // 内容不足时：跟随则贴底，否则贴顶（保持阅读位置稳定）
        let follow = self.view.map(|v| v.follow()).unwrap_or(true);
        let top = if follow { body.saturating_sub(shown.len()) } else { 0 };

        // 搜索命中行（用于高亮与当前命中标记）
        let hit_set: (Option<usize>, Vec<usize>) = match self.view.and_then(|v| v.search()) {
            Some(s) => (Some(s.current), s.hits.clone()),
            None => (None, Vec::new()),
        };

        for (i, segs) in shown.iter().enumerate() {
            let abs = start + i;
            let is_hit = hit_set.1.contains(&abs);
            let is_current = hit_set
                .0
                .and_then(|c| hit_set.1.get(c).copied())
                .map(|l| l == abs)
                .unwrap_or(false);
            for (col, text, tone) in segs {
                // 命中行加左侧标记：搜索"找到了"必须看得见
                let tone = if is_current {
                    Tone::Warning
                } else if is_hit {
                    Tone::Success
                } else {
                    *tone
                };
                g.put(top + i, *col, text, tone);
            }
            if is_hit {
                let mark = if is_current { "▶" } else { "│" };
                // 画在正文最右侧（不压字），窄终端下省略
                let x = self.body_cols().saturating_sub(2);
                if x > 4 {
                    g.put(
                        top + i,
                        x,
                        mark,
                        if is_current { Tone::Warning } else { Tone::Success },
                    );
                }
            }
        }

        // 滚动指示：不在底部时提示"下方还有内容"，避免用户以为到底了。
        // 位置固定为正文区**最后一行**（而不是"已显示内容的下一行"）——
        // 后者在贴底布局下会落到正文区之外，提示就永远看不见。
        if let Some(v) = self.view {
            let off = v.clamped_offset(total, body);
            if off > 0 && body > 0 {
                let msg = format!("↓ 下方还有 {off} 行（ctrl+e 到底）");
                g.put(body - 1, 2, &msg, Tone::Muted);
            }
        }

        let top = self.rows - chrome_rows(self.input.line_count());
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
        for &(_logo, subtitle, meta, gaps) in &tiers {
            let hero = self.hero_lines(a, true, subtitle, meta, gaps);
            // 整组要放得下，否则会被"显示末尾 N 行"从**顶部**裁掉 ——
            // 用户看到的是"少了 logo 的半截首屏"，且不会有任何报错
            if hero.len() + chrome_rows(1) <= self.rows {
                chosen = hero;
                break;
            }
        }
        if chosen.is_empty() {
            chosen = self.hero_lines(a, false, false, false, false);
        }

        let group = chosen.len() + chrome_rows(self.input.line_count());
        let group_top = self.rows.saturating_sub(group) / 2;
        for (i, segs) in chosen.iter().enumerate() {
            g.put_centered_styled(group_top + i, std::slice::from_ref(segs));
        }
        let top = group_top + chosen.len();
        (top, self.draw_chrome(g, top))
    }

    fn hero_lines(&self, a: &About, logo: bool, subtitle: bool, meta: bool, gaps: bool) -> Vec<Styled> {
        let mut hero: Vec<Styled> = Vec::new();
        // Logo 由**样式**决定，而不是"够宽就画大的" —— 后者让用户无法选择。
        // 样式为 Large 且终端够宽时画 6 行大词标；否则按样式退到小/极简。
        let t = theme::get(self.theme);
        let (pr, ac) = (t.primary, t.accent);
        let grad = |n: usize| -> Vec<Tone> {
            (0..n)
                .map(|i| {
                    let f = if n <= 1 { 0.0 } else { i as f32 / (n - 1) as f32 };
                    let (r, g, b) = lerp_rgb(pr, ac, f);
                    Tone::Rgb(r, g, b)
                })
                .collect()
        };
        match self.appearance.logo {
            appearance::LogoStyle::Hidden => {}
            appearance::LogoStyle::Minimal => {
                hero.push(("NEO".to_string(), Tone::Primary));
            }
            appearance::LogoStyle::Small if self.cols >= 24 => {
                let tones = grad(appearance::LOGO_SMALL.len());
                for (i, row) in appearance::LOGO_SMALL.iter().enumerate() {
                    hero.push((row.to_string(), tones[i]));
                }
            }
            appearance::LogoStyle::Small => {
                hero.push(("NEO".to_string(), Tone::Primary));
            }
            appearance::LogoStyle::Large if self.cols >= WORDMARK_MIN_COLS && logo => {
                let tones = grad(WORDMARK.len());
                for (i, row) in WORDMARK.iter().enumerate() {
                    hero.push((row.trim_end().to_string(), tones[i]));
                }
            }
            appearance::LogoStyle::Large => {
                // 大词标放不下（或首屏降级）→ 退到小词标，而不是不画
                if self.cols >= 24 {
                    let tones = grad(appearance::LOGO_SMALL.len());
                    for (i, row) in appearance::LOGO_SMALL.iter().enumerate() {
                        hero.push((row.to_string(), tones[i]));
                    }
                } else {
                    hero.push(("NEO".to_string(), Tone::Primary));
                }
            }
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

        // 整块铺**面板底色**：这才是"分栏"的层级感来源。
        // 只画一条竖线只能表达边界，表达不了"这是一块独立的区域"——
        // 用户说"没有层级边界感"主要指的就是这个。
        g.fill_bg(0, self.rows, x0, self.cols, Bg::Panel);
        // 左侧竖线保留：底色在浅色终端下可能不够明显，线是双保险
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
                // 无分支时退回显示模型名 —— 同样用实时值（见上面 2308 的注释）
                let label = if a.branch.is_empty() {
                    a.current_model.clone()
                } else {
                    a.branch.clone()
                };
                let t = width::truncate_to_width(&label, inner).to_string();
                g.put(self.rows - 2, x0 + 2, &t, Tone::Border);
            }
        }
    }

    /// 弹窗候选所在的行（供鼠标命中）。几何必须与 `draw_popup` 一致。
    fn popup_item_rows(&self, chrome_top: usize) -> Vec<((usize, usize, usize), usize)> {
        let Some(pop) = self.popup else {
            return Vec::new();
        };
        let body = self.body_cols();
        let box_w = self.box_width();
        let left = body.saturating_sub(box_w) / 2;
        let max_rows = pop.kind.max_rows();
        let avail = chrome_top.saturating_sub(2);
        let hint_rows = if pop.is_empty() { 1 } else { 0 };
        let foot_rows = if pop.truncated { 1 } else { 0 };
        let for_items = avail.saturating_sub(3 + hint_rows + foot_rows);
        let visible = pop.items.len().min(max_rows).min(for_items);
        let height = 2 + visible + hint_rows + foot_rows;
        if visible == 0 || chrome_top < height + 1 {
            return Vec::new();
        }
        let top = chrome_top - height - 1;
        if pop.is_empty() {
            return Vec::new();
        }
        let start = pop.scroll_top(visible);
        pop.items
            .iter()
            .enumerate()
            .skip(start)
            .take(visible)
            .map(|(i, _)| ((top + 2 + (i - start), left, left + box_w), i))
            .collect()
    }

    /// 在设置行下方画内联候选列表；返回画完之后的行号。
    ///
    /// 缩进（列 6）与设置行的值列对齐 —— 视觉上"这个值展开成了这些选项"，
    /// 而不是另开一个框。超出行数上限时只画能放下的部分（宁可少画候选，
    /// 也不能把底部反馈行/图例顶掉）。
    fn draw_inline_picker(
        &self,
        g: &mut Grid,
        pk: &SettingsPicker,
        mut row: usize,
        content_max: usize,
    ) -> usize {
        // 留一行给"共 N 项 / 上下选择"的说明
        let budget = content_max.saturating_sub(row + 1);
        for (i, item) in pk.items.iter().enumerate() {
            if i >= budget {
                break;
            }
            let sel = i == pk.selected;
            let mark = if sel { "❯ " } else { "  " };
            let tone = if sel { Tone::Primary } else { Tone::Muted };
            let text = if item.detail.is_empty() {
                item.label.clone()
            } else {
                format!("{}  {}", item.label, item.detail)
            };
            let avail = self.cols.saturating_sub(10);
            let shown = width::truncate_to_width(&text, avail).to_string();
            g.put(row, 6, mark, tone);
            g.put(row, 8, &shown, tone);
            row += 1;
        }
        if budget > pk.items.len() {
            let hint = format!("↑↓ 选择 · enter 确认 · esc 取消（{} 项）", pk.items.len());
            g.put(row, 8, &hint, Tone::Border);
            row += 1;
        }
        row
    }

    /// 设置视图：分节列出可配置项与只读项。
    ///
    /// `regions.settings_rows` 会登记可操作行的 y 坐标，供鼠标点击命中 ——
    /// 与 `settings_cursor` 用同一坐标空间（只数可操作行），
    /// 这样"点第 k 项"与"光标移到第 k 项再回车"必然作用于同一项。
    fn draw_settings(
        &self,
        g: &mut Grid,
        sections: &[SettingSection],
        regions: &mut Regions,
    ) -> Option<(usize, usize)> {
        // 标题
        let title = "设置";
        g.put(0, 2, title, Tone::Text);
        let hint = "↑↓ 移动  enter 执行  q / esc 返回";
        if self.cols > 50 {
            let w = width::display_width(hint);
            g.put(0, self.cols.saturating_sub(w + 2), hint, Tone::Border);
        }
        let bar = "─".repeat(self.cols.saturating_sub(2));
        g.put(1, 1, &bar, Tone::Border);

        let label_w = 14usize;
        let mut row = 2usize;
        // 底部留两行：反馈行 + 图例行。内容必须停在这之前，
        // 否则最后几项会压在被反馈行覆盖的位置上。
        let content_max = self.rows.saturating_sub(2);
        // 高亮下标必须与 Enter 用**同一个坐标空间**：只数可操作行。
        //
        // 之前这里数的是**全部行**（含只读行），而 `settings_cursor` 只在
        // 可操作行间移动 —— 于是高亮停在第 k 行、Enter 却作用在 `slots[k]`
        // （因只读行插在中间，通常更靠后）。表现就是：高亮停在只读行上，
        // 或"选中某一项按回车，动的是别的项 / 像没反应"。
        let mut act: usize = 0;
        for sec in sections {
            if row + 1 >= content_max {
                break;
            }
            row += 1; // 节前空行
            g.put(row, 2, sec.title, Tone::Accent);
            row += 1;
            for r in &sec.rows {
                if row >= content_max {
                    break;
                }
                let actionable = r.action.is_some();
                let selected = actionable && act == self.settings_cursor;
                if actionable {
                    // 登记行号 → 可操作序号（鼠标点击即用这个序号，等同于 Enter）
                    regions.settings_rows.push((row, act));
                    act += 1;
                }
                // 光标只停在可操作行上；只读行用弱色 + 说明
                let mark = if selected { "▸ " } else { "  " };
                if selected {
                    // 选中行铺高亮底色条（与审批卡片一致）——
                    // 光标位置只靠 ▸ 太弱，列表选中态应当"整行可见"
                    g.fill_bg(row, row + 1, 1, self.cols.saturating_sub(1), Bg::Selected);
                    g.put(row, 1, mark, Tone::Primary);
                }
                let lt = if actionable { Tone::Text } else { Tone::Muted };
                let padded = width::pad_to_width(&r.label, label_w);
                g.put(row, 3, &padded, lt);
                // 值
                let vt = if actionable {
                    Tone::Info
                } else {
                    Tone::Muted
                };
                let vx = 3 + label_w + 2;
                let note_w = if r.readonly_note.is_empty() {
                    self.cols.saturating_sub(vx + 2)
                } else {
                    width::display_width(&r.value).min(self.cols.saturating_sub(vx + 2))
                };
                let v = width::truncate_to_width(&r.value, note_w.max(4)).to_string();
                let vend = g.put(row, vx, &v, vt);
                // 只读原因（右侧，弱色）
                if !r.readonly_note.is_empty() {
                    let nx = vend + 2;
                    if nx < self.cols.saturating_sub(6) {
                        let note = width::truncate_to_width(
                            r.readonly_note,
                            self.cols.saturating_sub(nx + 2),
                        )
                        .to_string();
                        g.put(row, nx, &note, Tone::Border);
                    }
                }
                row += 1;
                // 内联选择器：就地长在这一行下面（不跳出设置页）。
                if let Some(pk) = self.settings_picker {
                    if pk.label == r.label {
                        row = self.draw_inline_picker(g, pk, row, content_max);
                    }
                }
            }
        }
        // 反馈行：动作做完必须让用户看见结果。
        //
        // 这是个真实踩过的坑：设置页原本把 Screen.status 硬编码成 ""，
        // 而"提醒 / 提醒声音 / 主题"这类动作**只改状态栏文案**、不改任何可见的值
        // —— 于是按回车后界面毫无变化，用户以为没反应。动作确实执行了，
        // 但界面没有给出任何信号，等于没有反馈。
        if self.rows > 3 && !self.status.is_empty() {
            let txt = width::truncate_to_width(self.status, self.cols.saturating_sub(4)).to_string();
            g.put(self.rows - 2, 2, &txt, Tone::Info);
        }
        // 底部：说明光标只能停在可操作行
        if self.rows > 2 {
            let hint = if self.settings_form.is_some() {
                "tab / ↑↓ 切字段 · enter 保存 · esc 取消"
            } else {
                "灰字为只读项（右侧是原因）；▸ 停在可操作项上"
            };
            g.put(self.rows - 1, 2, hint, Tone::Border);
        }
        // 确认条：**必须可见**，否则用户按了删除却不知道在等自己回答什么。
        if let Some(q) = self.settings_confirm {
            let txt = format!("{q}  [y 确认 / 其它键取消]");
            let txt = width::truncate_to_width(&txt, self.cols.saturating_sub(4)).to_string();
            // 用 Warning 色：这是"需要你回答"的状态，不是普通提示
            g.put(self.rows.saturating_sub(2), 2, &txt, Tone::Warning);
        }
        // 表单：设置页内的模态（新增/编辑服务商）。**最后画**，盖在列表之上。
        // 返回真实光标位置，否则输入时看不见插入点。
        self.draw_settings_form(g)
    }

    /// 画设置页里的表单；返回应放置硬件光标的位置。
    ///
    /// 密钥字段**打码显示**：明文铺在屏幕上，旁人一眼就能抄走，
    /// 而这是终端（可能共享屏幕/录屏）。编辑时也不回显明文。
    fn draw_settings_form(&self, g: &mut Grid) -> Option<(usize, usize)> {
        let form = self.settings_form?;
        let w = self.cols.saturating_sub(8).min(76).max(30);
        let fields = form.fields.len();
        let h = fields + 4; // 标题 + 分隔 + 字段 + 提示 + 边框
        let top = self.rows.saturating_sub(h).saturating_sub(2).max(2);
        let left = (self.cols.saturating_sub(w)) / 2;

        // 先清出整块矩形：不清的话下面的列表文字会从空隙里透出来
        for r in top..(top + h).min(self.rows) {
            g.blank(r, left, (left + w).min(self.cols - 1), Tone::Text);
        }
        let bar = "─".repeat(w.saturating_sub(2));
        g.put(top, left, "╭", Tone::Border);
        g.put(top, left + 1, &bar, Tone::Border);
        g.put(top, left + w - 1, "╮", Tone::Border);
        let title = width::truncate_to_width(&form.title, w.saturating_sub(4)).to_string();
        g.put(top, left + 2, &title, Tone::Accent);

        let label_w = 10usize;
        let mut cursor = None;
        for (i, f) in form.fields.iter().enumerate() {
            let r = top + 1 + i;
            g.put(r, left, "│", Tone::Border);
            g.put(r, left + w - 1, "│", Tone::Border);
            let active = i == form.active;
            let lt = if active { Tone::Primary } else { Tone::Muted };
            let mark = if active { "▸ " } else { "  " };
            let padded = width::pad_to_width(&f.label, label_w);
            g.put(r, left + 2, mark, lt);
            g.put(r, left + 4, &padded, lt);
            let vx = left + 4 + label_w + 1;
            let avail = (left + w - 2).saturating_sub(vx);
            let raw = f.editor.text();
            // 密钥打码：长度也模糊化（固定 8 个点），不泄露密钥长度
            let shown = if f.secret {
                if raw.is_empty() { String::new() } else { "••••••••".to_string() }
            } else {
                raw.clone()
            };
            let shown = width::truncate_to_width(&shown, avail).to_string();
            let vt = if active { Tone::Text } else { Tone::Muted };
            g.put(r, vx, &shown, vt);
            if active {
                // 光标列 = 字段值**显示宽度**（与输入框同一个换算教训：
                // 字符下标 ≠ 显示列，中文/打码都不等长）
                let (_, col) = f.editor.cursor();
                let before: String = raw.chars().take(col).collect();
                let cw = if f.secret {
                    // 打码后长度与原文无关：按已显示的点数定位（上限 8）
                    before.chars().count().min(8)
                } else {
                    width::display_width(&before)
                };
                let cx = (vx + cw).min(left + w - 2);
                cursor = Some((r + 1, cx + 1)); // 1 基
            }
        }
        let br = top + 1 + fields;
        g.put(br, left, "╰", Tone::Border);
        g.put(br, left + 1, &bar, Tone::Border);
        g.put(br, left + w - 1, "╯", Tone::Border);
        cursor
    }

    /// 审批对话框：居中的模态卡片。
    ///
    /// 内容刻意分三层，按"用户判断需要什么"排序：
    /// 1. **要做什么**（工具 + 参数摘要）—— 没有它用户是在盲批；
    /// 2. **改动预览**（若有）—— 决定放不放行的关键信息；
    /// 3. 三个选项（↑↓ 选、回车确认，y/a/n 直接生效）。
    fn draw_approval(&self, g: &mut Grid, ap: &ApprovalPrompt, chrome_top: usize) {
        let w = self.cols.saturating_sub(8).min(78).max(30);
        // 内容行先算出来，再据此定高 —— 之前用 `body.len()+choices+4` 多留了
        // 4 行，卡片底部因此挂着一块**没有左右边框**的空白（用户截图的现象）。
        // 高度必须由**实际要画的内容**决定。
        let mut body: Vec<(String, Tone)> = Vec::new();
        body.push((format!("要执行  {}", ap.call), Tone::Text));
        if !ap.detail.is_empty() {
            body.push((ap.detail.clone(), Tone::Warning));
        }
        if let Some((path, diff)) = &ap.diff {
            body.push((format!("改动  {path}"), Tone::Info));
            let lines: Vec<&str> = diff.lines().collect();
            const MAX_DIFF: usize = 8;
            for l in lines.iter().take(MAX_DIFF) {
                // 增删行用不同色调，扫一眼就知道改了什么
                let tone = if l.starts_with('+') && !l.starts_with("+++") {
                    Tone::Success
                } else if l.starts_with('-') && !l.starts_with("---") {
                    Tone::Error
                } else {
                    Tone::Muted
                };
                body.push((l.to_string(), tone));
            }
            if lines.len() > MAX_DIFF {
                body.push((format!("… 还有 {} 行（d 看完整 diff）", lines.len() - MAX_DIFF), Tone::Border));
            }
        }
        let choices = ApprovalPrompt::CHOICES.len();
        // 边框 2 + 内容 + 选项 + 标题下方一条分隔
        let h = body.len() + choices + 3;
        if h + 1 >= chrome_top || w < 30 {
            return; // 放不下就不画，宁可退回输入框路径
        }
        let top = chrome_top.saturating_sub(h) / 2;
        let left = (self.cols.saturating_sub(w)) / 2;
        let inner = w.saturating_sub(4);

        // 表面底色：卡片是"浮在正文之上的一层"，靠底色区分而不是靠更多边框
        g.fill_bg(top, top + h, left, left + w, Bg::Surface);
        // 先清字符（底色之上再写内容）
        for r in top..top + h {
            g.blank(r, left, (left + w).min(self.cols.saturating_sub(1)), Tone::Text);
        }
        let bar = "─".repeat(w.saturating_sub(2));
        // 圆角边框 + 醒目色：这是"需要你决定"的状态
        let bd = Tone::Warning;
        g.put(top, left, "╭", bd);
        g.put(top, left + 1, &bar, bd);
        g.put(top, left + w - 1, "╮", bd);
        let title = " 需要审批 ";
        g.put(top, left + 2, title, Tone::Warning);

        let mut row = top + 1;
        for (text, tone) in &body {
            g.put(row, left, "│", bd);
            let shown = width::truncate_to_width(text, inner).to_string();
            g.put(row, left + 2, &shown, *tone);
            // 右侧补边框：整行都要有左右边界，否则"面"不闭合
            g.put(row, left + w - 1, "│", bd);
            row += 1;
        }
        // 分隔线：把"看什么"与"选什么"分开（层级感的另一半）
        g.put(row, left, "│", bd);
        g.put(row, left + 1, &"·".repeat(w.saturating_sub(2)), Tone::Border);
        g.put(row, left + w - 1, "│", bd);
        row += 1;
        for (i, c) in ApprovalPrompt::CHOICES.iter().enumerate() {
            let sel = i == ap.selected;
            g.put(row, left, "│", bd);
            if sel {
                // 选中行给**底色条**（不是只加一个符号）—— 列表选中态在
                // opencode/mimo 里是一整条高亮，不是一个箭头
                g.fill_bg(row, row + 1, left + 1, left + w - 1, Bg::Selected);
            }
            let mark = if sel { "❯ " } else { "  " };
            let tone = if sel { Tone::Primary } else { Tone::Muted };
            g.put(row, left + 2, mark, tone);
            g.put(row, left + 4, c, tone);
            g.put(row, left + w - 1, "│", bd);
            row += 1;
        }
        g.put(row, left, "╰", bd);
        g.put(row, left + 1, &bar, bd);
        g.put(row, left + w - 1, "╯", bd);
    }

    /// which-key 覆盖层：右下的键位提示卡片。
    ///
    /// 放在**右下**而不是居中：用户通常还在打字，居中会盖住刚看的内容。
    fn draw_whichkey(&self, g: &mut Grid, groups: &[whichkey::Group], chrome_top: usize) {
        if groups.is_empty() || chrome_top < 4 {
            return;
        }
        // 计算卡片尺寸：键列 + 说明列
        let key_w = groups
            .iter()
            .flat_map(|g| g.keys.iter().map(|(k, _)| width::display_width(k)))
            .max()
            .unwrap_or(5);
        let desc_w = groups
            .iter()
            .flat_map(|g| g.keys.iter().map(|(_, d)| width::display_width(d)))
            .max()
            .unwrap_or(10);
        // 卡片宽度：边框 2 + 键列 + 间隔 2 + 说明列
        let card_w = (key_w + desc_w + 6).min(self.cols.saturating_sub(4)).max(20);
        // 高度：每个分组 1 行标题 + 若干键行 + 1 行间距
        let rows_needed: usize =
            groups.iter().map(|g| g.keys.len() + 2).sum::<usize>() + 1;
        let card_h = rows_needed.min(chrome_top.saturating_sub(1));
        if card_h < 3 {
            return;
        }
        let x0 = self.body_cols().saturating_sub(card_w + 1);
        let y0 = chrome_top.saturating_sub(card_h + 1);

        // 先填空整块（不透明），否则底下的正文会从缝隙里透上来
        for r in y0..(y0 + card_h).min(self.rows) {
            g.blank(r, x0, x0 + card_w, Tone::Text);
        }
        let bar = "─".repeat(card_w.saturating_sub(2));
        g.put(y0, x0, "╭", Tone::BorderActive);
        g.put(y0, x0 + 1, &bar, Tone::BorderActive);
        g.put(y0, x0 + card_w - 1, "╮", Tone::BorderActive);

        let mut row = y0 + 1;
        let bottom = y0 + card_h - 1;
        for grp in groups {
            if row >= bottom {
                break;
            }
            g.put(row, x0, "│", Tone::BorderActive);
            let t = width::truncate_to_width(grp.title, card_w.saturating_sub(4)).to_string();
            g.put(row, x0 + 2, &t, Tone::Accent);
            g.put(row, x0 + card_w - 1, "│", Tone::BorderActive);
            row += 1;
            for (k, d) in grp.keys {
                if row >= bottom {
                    break;
                }
                g.put(row, x0, "│", Tone::BorderActive);
                let kw = width::pad_to_width(k, key_w);
                g.put(row, x0 + 2, &kw, Tone::Primary);
                let dw = card_w.saturating_sub(key_w + 5);
                let desc = width::truncate_to_width(d, dw).to_string();
                g.put(row, x0 + 2 + key_w + 2, &desc, Tone::Muted);
                g.put(row, x0 + card_w - 1, "│", Tone::BorderActive);
                row += 1;
            }
            row += 1; // 分组间距
        }
        // 底部：补一条提示（说明这是 which-key 以及怎么关）
        if bottom > y0 {
            g.put(bottom, x0, "╰", Tone::BorderActive);
            g.put(bottom, x0 + 1, &bar, Tone::BorderActive);
            g.put(bottom, x0 + card_w - 1, "╯", Tone::BorderActive);
        }
    }

    /// 全屏 diff 查看器：左侧文件树（可关）+ 右侧 diff。
    fn draw_diff_viewer(&self, g: &mut Grid, p: &Pal, v: &diffview::Viewer) {
        let _ = p;
        if v.is_empty() {
            g.put_centered(1, "（无改动）", Tone::Muted);
            g.put(2, 2, "按 q / esc 返回", Tone::Border);
            return;
        }

        // 标题行：摘要 + 键位提示
        let title = width::truncate_to_width(&v.summary(), self.cols.saturating_sub(2)).to_string();
        g.put(0, 1, &title, Tone::Text);
        let keys = "j/k 移动  ]/[ hunk  n/p 文件  v 视图  b 树  q 返回";
        if self.cols > 70 {
            let kw = width::display_width(keys);
            g.put(0, self.cols.saturating_sub(kw + 1), keys, Tone::Border);
        }

        // 文件树（宽终端才显示）
        let tree_w = if v.tree && self.cols >= 90 { 30usize } else { 0 };
        let diff_x = if tree_w > 0 { tree_w + 1 } else { 0 };
        if tree_w > 0 {
            g.put(1, 0, &"─".repeat(tree_w), Tone::Border);
            g.put(1, tree_w, "┬", Tone::Border);
            for (i, f) in v.diff.files.iter().enumerate().take(self.rows.saturating_sub(3)) {
                let r = 2 + i;
                if r >= self.rows - 1 {
                    break;
                }
                let selected = i == v.file_cursor;
                let mark = if selected { "▸ " } else { "  " };
                let counts = format!("+{} -{}", f.adds, f.dels);
                let budget = tree_w.saturating_sub(2 + counts.len() + 2);
                let name = truncate_left(&f.path, budget.max(4));
                let tone = if selected { Tone::Primary } else { Tone::Muted };
                g.put(r, 0, mark, Tone::Primary);
                let end = g.put(r, 2, &name, tone);
                let cx = tree_w.saturating_sub(counts.len() + 1);
                if cx > end + 1 {
                    g.put(r, cx, &counts, Tone::Muted);
                }
            }
        }
        // 分隔竖线
        for r in 1..self.rows {
            if tree_w > 0 {
                g.put(r, tree_w, "│", Tone::Border);
            }
        }

        // diff 内容
        let viewport = self.rows.saturating_sub(3); // 顶部标题 + 底部提示 + 边距
        let start = v.offset.min(v.line_count().saturating_sub(1));
        let avail_w = self.cols.saturating_sub(diff_x + 2);
        match v.mode {
            diffview::ViewMode::Unified => {
                for (i, line) in v.diff.lines.iter().enumerate().skip(start).take(viewport) {
                    let r = 2 + (i - start);
                    if r >= self.rows - 1 {
                        break;
                    }
                    let is_cursor = i == v.cursor;
                    self.put_diff_line(g, r, diff_x, line, avail_w, is_cursor);
                }
            }
            diffview::ViewMode::Split => {
                // 双列：左边删除/上下文，右边新增/上下文。
                // 这不是严格的"左右对齐 diff"（那要按 hunk 配对），而是
                // 把删除与新增分列展示 —— 宽终端下读改动的常见需求。
                let half = avail_w / 2;
                let mut left_row = 2usize;
                let mut right_row = 2usize;
                for (i, line) in v.diff.lines.iter().enumerate().skip(start) {
                    if left_row >= self.rows - 1 && right_row >= self.rows - 1 {
                        break;
                    }
                    let is_cursor = i == v.cursor;
                    match line.kind {
                        diffview::Kind::Del => {
                            if left_row < self.rows - 1 {
                                self.put_diff_line(g, left_row, diff_x, line, half, is_cursor);
                                left_row += 1;
                            }
                        }
                        diffview::Kind::Add => {
                            if right_row < self.rows - 1 {
                                self.put_diff_line(
                                    g, right_row, diff_x + half, line, half, is_cursor,
                                );
                                right_row += 1;
                            }
                        }
                        _ => {
                            if left_row < self.rows - 1 {
                                self.put_diff_line(g, left_row, diff_x, line, half, is_cursor);
                                left_row += 1;
                            }
                            if right_row < self.rows - 1 {
                                self.put_diff_line(
                                    g, right_row, diff_x + half, line, half, is_cursor,
                                );
                                right_row += 1;
                            }
                        }
                    }
                }
            }
        }

        // 底部提示
        let more = v.line_count().saturating_sub(start + viewport);
        let foot = if more > 0 {
            format!("↓ 下方还有 {more} 行 · 共 {} 行", v.line_count())
        } else {
            format!("共 {} 行", v.line_count())
        };
        g.put(self.rows - 1, diff_x + 1, &foot, Tone::Border);
    }

    /// 画一行 diff（含行号、标记、色调）。
    fn put_diff_line(
        &self,
        g: &mut Grid,
        row: usize,
        x: usize,
        line: &diffview::Line,
        avail_w: usize,
        is_cursor: bool,
    ) {
        if avail_w < 6 {
            return;
        }
        // 光标行整行反白（用 Primary 前景 + 标记表明当前位置）
        let (sign, tone) = match line.kind {
            diffview::Kind::Header => ("", Tone::Muted),
            diffview::Kind::HunkHeader => ("", Tone::Accent),
            diffview::Kind::Add => ("+", Tone::Success),
            diffview::Kind::Del => ("-", Tone::Error),
            diffview::Kind::Context => (" ", Tone::Muted),
        };
        let tone = if is_cursor { Tone::Primary } else { tone };
        let cursor_mark = if is_cursor { "▌" } else { " " };
        g.put(row, x, cursor_mark, Tone::Primary);
        // 行号：旧号在左、新号在右（宽度 4+4）
        // 行号的 `{:>4}` 需要实参；空位用空串占位（宽度靠格式化保证）
        // 分别格式化两侧再拼 —— 单条 format 里混用有/无参数容易出错
        let left = line.old_no.map(|o| format!("{o:>4}")).unwrap_or_else(|| "    ".into());
        let right = line.new_no.map(|n| format!("{n:>4}")).unwrap_or_else(|| "    ".into());
        let nos = format!("{left} {right}");
        g.put(row, x + 1, &nos, Tone::Muted);
        g.put(row, x + 10, sign, tone);
        let text_w = avail_w.saturating_sub(12);
        let text = width::truncate_to_width(&line.text, text_w).to_string();
        g.put(row, x + 12, &text, tone);
    }

    /// 弹窗：标题 + 候选项 + （截断时）页脚。画在输入框上方。
    fn draw_popup(&self, g: &mut Grid, p: &Pal, pop: &popup::Popup, chrome_top: usize) {
        let _ = p;
        let body = self.body_cols();
        let box_w = self.box_width();
        let left = body.saturating_sub(box_w) / 2;
        let inner = box_w.saturating_sub(4);
        let max_rows = pop.kind.max_rows();
        // 上方可用空间：chrome_top 上面还要留 2 行（边框 + 与输入框的间隔）。
        // 装不下时**收缩可见行数**而不是不画 —— 不画的话用户按了 `/` 却
        // 什么都没出现，看起来像功能坏了（矮终端下必然发生）。
        let avail = chrome_top.saturating_sub(2);
        let hint_rows = if pop.is_empty() { 1 } else { 0 };
        let foot_rows = if pop.truncated { 1 } else { 0 };
        // 需要：2(边框) + 标题(1) + 候选 + 分组标题 + hint + foot
        let for_items = avail.saturating_sub(3 + hint_rows + foot_rows);
        // **分组标题也占行** —— 不把它算进来的话，实际内容会超出框，
        // 超出的那几行画在未清空的格子上，底下界面的文字就会透上来
        //（实测看到 `/undo` 那行前面挂着欢迎页的 `v0.1`）。
        // 用不动点收敛：先按无标题估，再按"标题占掉的行"收缩，直到装得下。
        let show_group = pop.kind == popup::Kind::Palette || pop.kind == popup::Kind::Slash;
        let mut visible = pop.items.len().min(max_rows).min(for_items);
        if show_group {
            loop {
                let headers = group_headers(&pop.items, visible);
                if visible + headers <= for_items || visible == 0 {
                    break;
                }
                visible -= 1;
            }
        }
        if available_rows(avail) == 0 || pop.items.len() > 0 && visible == 0 {
            // 连一行候选都放不下：至少把标题画出来，让用户知道弹窗开了
            let height = 3 + hint_rows;
            if chrome_top < height + 1 {
                return;
            }
            let top = chrome_top - height - 1;
            let body = self.body_cols();
            let box_w = self.box_width();
            let left = body.saturating_sub(box_w) / 2;
            for r in top..(top + height).min(self.rows) {
                g.blank(r, left, left + box_w, Tone::Text);
            }
            let bar = "─".repeat(box_w.saturating_sub(2));
            g.put(top, left, "╭", Tone::BorderActive);
            g.put(top, left + 1, &bar, Tone::BorderActive);
            g.put(top, left + box_w - 1, "╮", Tone::BorderActive);
            g.put(top + 1, left, "│", Tone::BorderActive);
            let title = format!("{} · {}", pop.kind.title(), pop.query);
            let t = width::truncate_to_width(&title, box_w.saturating_sub(4)).to_string();
            g.put(top + 1, left + 2, &t, Tone::Muted);
            g.put(top + 1, left + box_w - 1, "│", Tone::BorderActive);
            g.put(top + 2, left, "╰", Tone::BorderActive);
            g.put(top + 2, left + 1, &bar, Tone::BorderActive);
            g.put(top + 2, left + box_w - 1, "╯", Tone::BorderActive);
            return;
        }
        let headers = if show_group { group_headers(&pop.items, visible) } else { 0 };
        let height = 2 + visible + headers + hint_rows + foot_rows; // 含上下边框这 2 行
        if chrome_top < height + 1 {
            return;
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
            // 面板带分组时，在**第一个可见项**上方标一次组名（滚动中会随项移动，
            // 这让"我现在在哪个组"始终可见，而不是只在顶部标一次）。
            for (i, item) in pop.items.iter().enumerate().skip(start).take(visible) {
                if row >= bottom_limit(top, height) {
                    break;
                }
                let selected = i == pop.selected;
                // 分组标题（仅当与上一项不同、且是真分组）
                if show_group && !item.category.is_empty() {
                    let prev = if i > 0 { pop.items.get(i - 1).map(|x| x.category) } else { None };
                    if prev != Some(item.category) {
                        if row + 1 < height {
                            g.put(top + row, left, "│", Tone::BorderActive);
                            g.put(
                                top + row,
                                left + 2,
                                item.category,
                                Tone::Accent,
                            );
                            g.put(top + row, left + box_w - 1, "│", Tone::BorderActive);
                            row += 1;
                        }
                    }
                }
                let (mark, label_tone) = if selected {
                    ("▸ ", Tone::Primary)
                } else {
                    ("  ", Tone::Text)
                };
                // 选中标记：把左侧边框换成实心竖条 + 标记前缀高亮。
                // 不做整行反白 —— 网格只有前景色，反白会让文字与背景同色（看不见）。
                g.put(top + row, left, if selected { "┃" } else { "│" },
                      if selected { Tone::Primary } else { Tone::BorderActive });
                g.put(top + row, left + 2, mark, Tone::Primary);
                // 右侧优先放键位：它比描述短，且"能与不能按"更值得一眼看到
                let kb = item.keybinding;
                let kb_w = width::display_width(kb);
                let kb_x = if kb.is_empty() {
                    None
                } else {
                    let x = (left + box_w - 3).saturating_sub(kb_w);
                    if x > left + 8 {
                        Some(x)
                    } else {
                        None
                    }
                };
                let label_budget = match kb_x {
                    Some(x) => x.saturating_sub(left + 4).saturating_sub(1),
                    None => inner.saturating_sub(2),
                };
                let label = width::truncate_to_width(&item.label, label_budget.max(4)).to_string();
                let used = g.put(top + row, left + 4, &label, label_tone);
                if let Some(x) = kb_x {
                    g.put(top + row, x, kb, Tone::Muted);
                } else if !item.detail.is_empty() {
                    // 没键位时退而给描述
                    let d = width::truncate_to_width(&item.detail, 30).to_string();
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
        // 审批对话框（模态）在场时：输入框**不**抢焦点 —— 不变黄、不显示
        // "y 批准/n 拒绝"前缀。之前三处同时喊审批（卡片 + 输入框 + 状态行），
        // 用户看到的是一堆重复提示，反而不知道在哪答。
        let modal = self.approval.is_some();
        let awaiting = self.awaiting_input && !modal;
        let border = if awaiting { Tone::Warning } else { Tone::BorderActive };
        let bar = "─".repeat(box_w.saturating_sub(2));

        g.put(top, left, "╭", border);
        g.put(top, left + 1, &bar, border);
        g.put(top, left + box_w - 1, "╮", border);

        // ── 输入区：最多 MAX_INPUT_ROWS 行，超出则显示末尾并提示 ──
        let total = self.input.line_count();
        let shown = total.min(MAX_INPUT_ROWS);
        let first = total - shown; // 显示最后 N 行（光标总在可见区内）
        let (crow, ccol) = self.input.cursor();

        // 先把输入区整片填空，避免星场渗进框里
        let area_bottom = top + 1 + shown + 1;
        for r in (top + 1)..area_bottom.min(self.rows) {
            g.blank(r, left + 1, left + box_w - 1, Tone::Text);
        }

        for (i, line) in self.input.lines().iter().enumerate().skip(first).take(shown) {
            let r = top + 1 + (i - first);
            g.put(r, left, "│", border);
            if line.is_empty() && total == 1 && !awaiting {
                // 空输入：给占位提示 + 示例（否则光标处一片空白，不知道能打什么）
                let ex = match self.about {
                    Some(a) if !a.example.is_empty() => a.example.clone(),
                    _ => "输入任务".to_string(),
                };
                let hint = if awaiting {
                    "y 批准 / a 总是 / n 拒绝 > ".to_string()
                } else {
                    format!("输入任务… 例：{ex}")
                };
                let hint = width::truncate_to_width(&hint, inner_w).to_string();
                g.put(r, left + 2, &hint, Tone::Muted);
            } else {
                let prefix = if awaiting && i == first {
                    "y 批准 / a 总是 / n 拒绝 > ".to_string()
                } else {
                    String::new()
                };
                // 截断到框内宽度：不截的话长输入会盖掉右边框
                // （网格的 put 只受 body/sidebar 边界约束，不知道"框"的右边界）
                let text = format!("{prefix}{line}");
                let text = width::truncate_to_width(&text, inner_w).to_string();
                g.put(r, left + 2, &text, Tone::Text);
            }
            g.put(r, left + box_w - 1, "│", border);
        }
        // 行数超上限：在最后一行右侧标注（不静默）
        if total > shown {
            let more = format!("… 共 {total} 行 ");
            let mw = width::display_width(&more);
            let r = top + shown;
            if left + box_w > mw + 4 {
                g.put(r, left + box_w - mw - 2, &more, Tone::Muted);
            }
        }

        // 内层状态行（对标 MiMo 输入框内的 "Build ⏵ 模型"）
        let stat_row = top + 1 + shown;
        g.put(stat_row, left, "│", border);
        let inner = match self.about {
            // 用 `current_model`（实时）而不是 `model`（启动快照）——
            // 否则切换模型后输入框下方一直显示启动时那个名字。
            // 这是**第三次**踩同一个坑（会话 id、设置页模型行、这里），
            // 已把它当纪律：显示"会变的状态"一律用实时值。
            Some(a) if !a.mode_short.is_empty() => {
                format!("{} ⏵ {}", a.mode_short, a.current_model)
            }
            Some(a) => a.current_model.clone(),
            None => String::new(),
        };
        g.put(
            stat_row,
            left + 2,
            &width::truncate_to_width(&inner, inner_w).to_string(),
            Tone::Muted,
        );
        g.put(stat_row, left + box_w - 1, "│", border);

        let bottom = stat_row + 1;
        g.put(bottom, left, "╰", border);
        g.put(bottom, left + 1, &bar, border);
        g.put(bottom, left + box_w - 1, "╯", border);

        // 提示行：与输入框左右对齐
        let hint_row = bottom + 1;
        if hint_row < self.rows {
            g.put(hint_row, left + 2, HINT_LEFT, Tone::Border);
            let hw = width::display_width(HINT_RIGHT);
            let hx = (left + box_w).saturating_sub(2 + hw);
            if hx > left + 2 + width::display_width(HINT_LEFT) {
                g.put(hint_row, hx, HINT_RIGHT, Tone::Border);
            }
        }

        // 状态行（最底）：左 = 工作区:分支，右 = 状态文本
        let status_row = hint_row + 1;
        if status_row < self.rows {
            let ws_line = match self.about {
                Some(a) if !a.branch.is_empty() => format!("{}:{}", a.workspace, a.branch),
                Some(a) => a.workspace.clone(),
                None => String::new(),
            };
            let ws_shown =
                width::truncate_to_width(&ws_line, body.saturating_sub(6)).to_string();
            let wsw = width::display_width(&ws_shown);
            let stw = width::display_width(self.status);
            if self.status.is_empty() || wsw + stw + 4 > body {
                g.put(status_row, 2, &ws_shown, Tone::Dim);
            } else {
                g.put(status_row, 2, &ws_shown, Tone::Dim);
                let tone = if awaiting { Tone::Warning } else { Tone::Muted };
                // 模态在场时状态行不再重复"待审批…"—— 卡片已经是唯一入口。
                // 三处同时提示只会让人不知道在哪答。
                if !modal || !self.status.contains("审批") {
                    g.put(status_row, body - stw - 2, self.status, tone);
                }
            }
        }

        if self.show_cursor {
            // 光标位置（1 基）：落在实际光标行列上，而不是"文本末尾"。
            let vis_row = crow.saturating_sub(first).min(shown.saturating_sub(1));
            let prefix_w = if awaiting && crow == first {
                width::display_width("y 批准 / a 总是 / n 拒绝 > ")
            } else {
                0
            };
            // 光标前的列偏移必须按**显示宽度**算，不能用字符个数。
            //
            // `editor.cursor()` 的列是**字符下标**（这样编辑逻辑与字节/宽度解耦，
            // 中文按 1 个字符算）。但终端里一个汉字占 **2 列**，直接把字符下标
            // 当列偏移会偏小：输入「你是谁」后光标列=3，实际应落在第 6 列，
            // 按 3 算就落在了第二个字「是」的中间 —— 正是用户截图里的现象。
            let cur_line = self.input.lines().get(crow).map(String::as_str).unwrap_or("");
            let before: String = cur_line.chars().take(ccol).collect();
            let ccol_w = width::display_width(&before);
            // 再夹到输入框内宽：行被截断时字符偏移可能超出可视区，
            // 不夹的话光标会跑到右边框外面（比位置偏一点更糟）。
            let max_w = inner_w.saturating_sub(prefix_w);
            let ccol_w = ccol_w.min(max_w);
            Some((top + 2 + vis_row, left + 3 + prefix_w + ccol_w))
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
    /// 把事实渲染成"行 → 片段"。实现见同名的自由函数 ——
    /// **渲染与搜索必须共用同一份行生成**，否则命中行号会与实际渲染错位。
    fn fact_lines(&self) -> Vec<Vec<Seg>> {
        fact_lines_with(self.facts, self.body_cols(), self.display)
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
/// 从事件流里构造审批框内容（最后一个 ApprovalRequest + 它的调用与预览）。
///
/// 内核会把"待审批的调用"和它的改动预览一起发出来；这里把它们聚成
/// 一张卡片的内容 —— 用户要看到"批的是什么"，而不是只有一句
/// "写入类调用需确认"。
fn build_approval_prompt(events: &[EventMsg]) -> Option<ApprovalPrompt> {
    let detail = events.iter().rev().find_map(|e| match e {
        EventMsg::ApprovalRequest { detail, .. } => Some(detail.clone()),
        _ => None,
    })?;
    // 紧邻的 PatchProposed 是本次要批的改动（内核在审批前发出）
    let diff = events.iter().rev().find_map(|e| match e {
        EventMsg::PatchProposed { path, diff } => Some((path.clone(), diff.clone())),
        _ => None,
    });
    // 最后一个工具调用 = 待批的那个（参数摘要让用户知道在批什么）
    let call = events
        .iter()
        .rev()
        .find_map(|e| match e {
            EventMsg::ToolCallBegin { name, arguments, .. } => {
                let brief = arguments
                    .get("cmd")
                    .or_else(|| arguments.get("path"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(if brief.is_empty() { name.clone() } else { format!("{name} {brief}") })
            }
            _ => None,
        })
        .unwrap_or_else(|| "(未知调用)".to_string());
    Some(ApprovalPrompt { detail, call, diff, selected: 0 })
}

/// 就当前审批做出决定并提交给内核，若还有下一个审批则重建卡片。
///
/// 抽出来是为了让"快捷键"与"回车确认"走**同一条**提交路径 ——
/// 两处各写一遍会漂移出"快捷键批了但状态没更新"这类问题。
fn decide_approval<F>(
    approval: &mut Option<ApprovalPrompt>,
    outstanding: &mut Option<String>,
    events: &mut Vec<EventMsg>,
    submit: &mut F,
    decision: neo_protocol::Decision,
) where
    F: FnMut(neo_protocol::Op) -> Result<Vec<EventMsg>, String>,
{
    *approval = None;
    let Some(id) = outstanding.take() else { return };
    match submit(neo_protocol::Op::Approve { id, decision }) {
        Ok(produced) => {
            *outstanding = latest_approval_id(&produced);
            events.extend(produced);
        }
        Err(e) => events.push(EventMsg::Error { message: e }),
    }
}

/// 审批应答：允许一次 / 总是允许这类 / 拒绝。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalAnswer {
    Allow,
    AllowAlways,
    Deny,
}

/// 解析审批输入。`a` = always（总是允许**这一类**），沿用 Unix `y/n/a` 的习惯。
///
/// 为什么必须有"总是允许"：之前只有 y/n，于是模型每调一次工具就重新问一遍
/// （真实反馈：连按十几次 y 仍在问）。用户想要的显然是"这类别再问了"。
pub fn parse_approval_answer(s: &str) -> Option<ApprovalAnswer> {
    match s.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => Some(ApprovalAnswer::Allow),
        "a" | "always" | "all" => Some(ApprovalAnswer::AllowAlways),
        "n" | "no" => Some(ApprovalAnswer::Deny),
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
        popup::Kind::Background => {
            let n = p.query.clone();
            p.set_items(popup::background_items(&n), false);
        }
        popup::Kind::Logo => {
            let n = p.query.clone();
            p.set_items(popup::logo_items(&n), false);
        }
        // 模型/会话列表由调用方装配（refresh 拿不到 about 与 SessionControl）
        popup::Kind::Models => {}
        popup::Kind::Sessions => {}
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
    /// 打开 diff 查看器
    ShowDiff,
    /// 回退对话一轮（需要向内核提交 Op，故交给主循环执行）
    Rewind,
    /// 切换工具输出展开 / 折叠
    ToggleDetails,
    /// 切换推理显隐
    ToggleThinking,
    /// 复制最近一条助手回复
    CopyLastReply,
    /// 打开设置
    Settings,
    /// 收起 / 展开侧栏
    ToggleSidebar,
    /// 开 / 关提醒
    ToggleNotify,
    /// 开 / 关提醒声音
    ToggleNotifySound,
    /// 切到下一个背景
    NextBackground,
    /// 打开背景选择列表
    BackgroundPicker,
    /// 切到下一个 Logo 样式
    NextLogo,
    /// 打开 Logo 样式选择列表
    LogoPicker,
    /// 应用背景（来自选择列表）
    SetBackground(appearance::Background),
    /// 应用 Logo 样式（来自选择列表）
    SetLogo(appearance::LogoStyle),
    /// 打开模型选择列表
    ModelPicker,
    /// 切换模型（提交给内核）
    SwitchModel(String),
    /// 打开会话列表
    SessionPicker,
    /// 切换到指定会话（由调用方的 SessionControl 执行）
    SwitchSession(String),
    /// 新建会话
    NewSessionReal,
    /// 删除会话
    DeleteSession(String),
    /// 打开某服务商的编辑表单
    EditProvider(String),
    /// 删除某服务商（候选列表里带删除项，或编辑表单里删）
    RemoveProvider(String),
    /// 提交压缩请求给内核
    CompactSubmit,
}

/// 执行弹窗里选中的项。返回需要主循环落实的副作用。
fn apply_popup_item(
    item: &popup::Item,
    input: &mut editor::Editor,
    status: &mut String,
    theme_name: &mut theme::ThemeName,
    info_screen: &mut Option<String>,
) -> Effect {
    match &item.action {
        popup::ItemAction::Insert(text) => {
            // 用选中的引用替换掉 `@` 之后已输入的过滤词
            let done = complete_at_token(&input.text(), text);
            input.set(&done);
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
        // 背景/Logo 由主循环落盘（apply_popup_item 拿不到外观状态）
        popup::ItemAction::SetBackground(b) => {
            input.clear();
            *status = format!("背景：{}", b.as_str());
            Effect::SetBackground(*b)
        }
        popup::ItemAction::SetLogo(l) => {
            input.clear();
            *status = format!("Logo：{}", l.as_str());
            Effect::SetLogo(*l)
        }
        popup::ItemAction::SwitchModel(name) => {
            input.clear();
            *status = format!("正在切换到 {name}…");
            Effect::SwitchModel(name.clone())
        }
        popup::ItemAction::SwitchSession(id) => {
            input.clear();
            Effect::SwitchSession(id.clone())
        }
        popup::ItemAction::DeleteSession(id) => {
            input.clear();
            Effect::DeleteSession(id.clone())
        }
        // 服务商编辑交给调用方（打开表单需要 ProviderControl，这里拿不到）
        popup::ItemAction::RunProvider(name) => Effect::EditProvider(name.clone()),
        popup::ItemAction::DeleteProvider(name) => Effect::RemoveProvider(name.clone()),
        popup::ItemAction::Run(action) => {
            // 执行命令后必须清空输入框：`/help` 已经"用掉"了，
            // 留在框里会让用户以为还没执行，且盖住占位提示。
            // （文件引用项相反 —— 那是要保留在输入里发给模型的。）
            input.clear();
            match action {
            commands::Action::Quit => Effect::Quit,
            commands::Action::NewSession => Effect::ClearTranscript,
            commands::Action::Compact => {
                // 提交给内核（内核实现在 L4 注入的策略上）。
                // 结果由主循环的 Effect::CompactSubmit 落实。
                Effect::CompactSubmit
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
            commands::Action::DiffViewer => Effect::ShowDiff,
            // 回退要提交 Op 给内核，因此交给主循环执行
            commands::Action::Rewind => Effect::Rewind,
            commands::Action::ToggleDetails => Effect::ToggleDetails,
            commands::Action::ToggleThinking => Effect::ToggleThinking,
            commands::Action::CopyLastReply => Effect::CopyLastReply,
            commands::Action::Settings => Effect::Settings,
            commands::Action::ToggleSidebar => Effect::ToggleSidebar,
            commands::Action::ToggleNotify => Effect::ToggleNotify,
            commands::Action::ToggleNotifySound => Effect::ToggleNotifySound,
            commands::Action::NextBackground => Effect::NextBackground,
            commands::Action::BackgroundPicker => Effect::BackgroundPicker,
            commands::Action::NextLogo => Effect::NextLogo,
            commands::Action::LogoPicker => Effect::LogoPicker,
            commands::Action::ModelPicker => Effect::ModelPicker,
            commands::Action::SessionPicker => Effect::SessionPicker,
            commands::Action::NewSessionReal => Effect::NewSessionReal,
            commands::Action::SwitchModel => Effect::None,
            // 这两个由选择列表内部产生（不注册命令）
            commands::Action::SetBackground(b) => Effect::SetBackground(*b),
            commands::Action::SetLogo(l) => Effect::SetLogo(*l),
            commands::Action::Keys => {
                *info_screen = Some(commands::keys_text().to_string());
                Effect::None
            }
            }
        }
    }
}

/// 信任对话框的按键结果。
enum TrustOutcome {
    /// 还需继续（选中项变了，需要重绘）
    Continue,
    /// 用户同意，写入信任库
    Accepted,
    /// 用户拒绝 / 退出
    Quit,
}

/// 处理信任对话框的一次按键。抽成函数是为了让"等待输入"与"渲染"两条路径
/// 共用同一套语义，避免各写一遍导致行为不一致。
fn handle_trust_key(k: Key, tp: &mut TrustPrompt, ws: &std::path::Path) -> TrustOutcome {
    match k {
        Key::Quit => TrustOutcome::Quit,
        Key::Up | Key::Char('k') => {
            tp.selected = 0;
            TrustOutcome::Continue
        }
        Key::Down | Key::Char('j') => {
            tp.selected = 1;
            TrustOutcome::Continue
        }
        Key::Char('y') => {
            accept_trust(ws);
            TrustOutcome::Accepted
        }
        Key::Char('n') => TrustOutcome::Quit,
        Key::Enter => {
            if tp.selected == 0 {
                accept_trust(ws);
                TrustOutcome::Accepted
            } else {
                TrustOutcome::Quit
            }
        }
        _ => TrustOutcome::Continue,
    }
}

/// 记录信任。失败必须如实上报 —— 静默失败会表现为"每次都问"，
/// 用户会以为是自己没点对。
fn accept_trust(ws: &std::path::Path) {
    if let Err(e) = trust::trust(ws) {
        eprintln!("[neo] 无法记录信任（{e}）；本次继续，下次仍会询问");
    }
}

/// 把事实渲染成"行 → 片段"（不含 ANSI，色调交给网格统一展开）。
///
/// 视觉语言（对标 opencode / MiMo）：
///   - 用户消息带左侧竖条，与助手正文区分
///   - 助手正文不加框、顶格直排
///   - 工具调用成功 ✓ / 失败 ✗，细节（exit code）压暗
///   - 元信息一律 muted，不抢正文
///
/// **这是行生成的唯一事实源**：渲染、搜索、行数估算都走它。
/// 各算一次的话行号必然对不上（搜索高亮会标在无关的行上）。
fn fact_lines(facts: &[Fact], body_cols: usize) -> Vec<Vec<Seg>> {
    fact_lines_with(facts, body_cols, ToolDisplay::default())
}

/// 会话控制契据 —— 由调用方（CLI）实现，宿主只调用。
///
/// # 为什么由宿主定义契据
///
/// 与 `submit` 闭包同一个思路：**宿主不持有内核，也不知道内核长什么样**。
/// 它只知道"我要切换/新建/删除会话"，具体怎么重建内核是 CLI 的事。
/// 反过来（宿主直接操作内核）会让 TUI 依赖 L2，破坏"宿主只是消费者"。
///
/// # 为什么 `switch` 返回事件流
///
/// 切换后宿主需要**重画新的转录**。返回重建出的 `EventMsg` 流，宿主直接
/// 用它重建 Facts —— 宿主不需要知道"日志怎么读、历史怎么重建"。
pub trait SessionControl {
    /// 列出现有会话：(id, 标题, 记录数)。最近修改的在前。
    fn list(&self) -> Vec<(String, String, usize)>;

    /// 切换到指定会话；返回该会话的历史事件流。
    fn switch(&mut self, id: &str) -> Result<Vec<EventMsg>, String>;

    /// 新建会话；返回 (新 id, 空事件流)。
    fn create(&mut self) -> Result<String, String>;

    /// 删除会话；`Ok(false)` 表示本来就不存在。
    fn delete(&mut self, id: &str) -> Result<bool, String>;

    /// 当前会话 id。
    fn current(&self) -> String;
}

/// 服务商管理契据。
///
/// # 为什么是契据而不是让宿主直接依赖 neo-providers
///
/// 与 `SessionControl` 同一个理由：宿主**不持有配置**。它只把用户的编辑意图
/// 转成契据调用，真正的读写在 L3（neo-providers）。这样：
/// 1. 宿主可脱离文件系统单测（用假实现）；
/// 2. 密钥的读写策略（0600、只读用户级）只在一个地方实现，不会在宿主里被绕过。
pub trait ProviderControl {
    /// 列出服务商：(名字, 描述, 是否已配密钥)。
    fn list(&self) -> Vec<(String, String, bool)>;

    /// 当前**内核里可用**的模型：(名字, 描述, 是否可用于真实任务)。
    ///
    /// 与 `list()` 的区别：`list()` 是配置文件里的服务商，这里是内核
    /// 实际加载成功的模型（缺密钥的不会出现）。新增服务商后宿主据此
    /// **立刻刷新模型列表**，不必重启。
    fn models(&self) -> Vec<(String, String, bool)>;

    /// 当前选中的模型名（用于判断新模型是否已生效）。
    fn current_model(&self) -> String;

    /// 查一条的完整字段（供编辑时回填）：(base_url, model, 描述, 上下文上限)。
    fn get(&self, name: &str) -> Option<(String, String, String, u64)>;

    /// 新增或更新。`api_key` 为空 = 不改动已存的密钥（编辑时"留空即保持"）。
    ///
    /// 这个约定是刻意的：让用户能看到"已配置"而不必重新粘贴密钥
    /// （把密钥回显到终端里既不安全也没必要）。
    fn upsert(
        &mut self,
        name: &str,
        base_url: &str,
        model: &str,
        api_key: &str,
        description: &str,
        context_limit: u64,
        editing_original: Option<&str>,
    ) -> Result<(), String>;

    /// 删除。返回是否真的删掉了。
    fn delete(&mut self, name: &str) -> Result<bool, String>;
}

/// 设置视图的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingRow {
    /// 左侧标签
    pub label: String,
    /// 当前值（人可读）
    pub value: String,
    /// 可点击执行的动作；`None` 表示只读（我们改不了，如实标注）
    pub action: Option<SettingAction>,
    /// 只读项的原因（`action` 为 None 时必须给，说明为什么不能改）
    pub readonly_note: &'static str,
}

/// 设置项的动作（受限于内核实际能力，只有这些是能改的）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingAction {
    /// 切换主题
    NextTheme,
    /// 打开主题列表
    ThemePicker,
    /// 切换工具输出展开
    ToggleDetails,
    /// 切换推理显隐
    ToggleThinking,
    /// 切换侧栏
    ToggleSidebar,
    /// 回退对话
    Rewind,
    /// 清空转录
    NewSession,
    /// 真·新建会话（换 session id + 落新日志文件；旧会话可 /sessions 切回）
    NewSessionReal,
    /// 查看 diff
    DiffViewer,
    /// 开 / 关提醒
    ToggleNotify,
    /// 开 / 关提醒声音
    ToggleNotifySound,
    /// 切到下一个背景
    NextBackground,
    /// 切到下一个 Logo 样式
    NextLogo,
    /// 打开模型选择
    ModelPicker,
    /// 打开服务商列表（选择后可编辑/删除）
    ProviderPicker,
    /// 新增服务商（打开表单）
    ProviderAdd,
    /// 编辑当前选中的服务商
    ProviderEdit,
    /// 删除某个服务商（选中后需确认）
    ProviderDelete,
    /// 打开会话选择
    SessionPicker,
}

/// 设置页里的表单（新增/编辑服务商）。
///
/// 为什么在宿主里做输入而不另开一个"对话框"：与内联选择器同一个理由 ——
/// 设置页是全屏的，输入也该在同一页里完成，不再跳出去。
#[derive(Debug, Clone)]
pub struct SettingsForm {
    pub title: String,
    pub fields: Vec<FormField>,
    /// 当前字段下标（tab / ↑↓ 切换）
    pub active: usize,
    /// 哪个字段是密钥（渲染时打码）
    pub key_field: Option<usize>,
    /// 编辑已有服务商时的原名；`None` = 新增（改名时用于删除旧条目）
    pub editing: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FormField {
    pub label: String,
    pub editor: editor::Editor,
    /// 是否打码显示（密钥）
    pub secret: bool,
}

impl SettingsForm {
    pub fn new(title: impl Into<String>, fields: Vec<FormField>, key_field: Option<usize>) -> Self {
        Self { title: title.into(), fields, active: 0, key_field, editing: None }
    }

    pub fn value(&self, i: usize) -> String {
        self.fields.get(i).map(|f| f.editor.text()).unwrap_or_default()
    }

    /// 取值并去掉首尾空白。
    pub fn trimmed(&self, i: usize) -> String {
        self.value(i).trim().to_string()
    }
}

/// 设置页内联选择器的候选：某项后面的动作已由 `popup::ItemAction` 承载。
///
/// 设置视图的一个分组。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingSection {
    pub title: &'static str,
    pub rows: Vec<SettingRow>,
}

/// 设置页内的内联选择器：在某一行**下方就地展开**候选列表。
///
/// 与 `popup::Popup` 的区别只在"画在哪"：候选与选中逻辑复用同一套
/// `popup::Item` / `ItemAction`，但渲染位置贴在该行之下，不跳出去。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsPicker {
    /// 展开在哪一行（用标签定位 —— 行是每次重算的，存下标会失效）。
    pub label: String,
    pub items: Vec<popup::Item>,
    pub selected: usize,
}

/// 设置视图的运行时信息（由主循环注入 —— 宿主不自己读配置）。
#[derive(Debug, Clone)]
pub struct SettingsInfo {
    pub version: String,
    /// 二进制自身的构建时间（取自可执行文件 mtime），形如 `2026-09-11 17:45`。
    ///
    /// # 为什么需要它
    ///
    /// `version` 是编译期常量（`0.1.0`），**改了代码重编译它也不变**，
    /// 于是"我跑的到底是哪一版"无法判断 —— 真实踩过：用户 PATH 里的 `neo`
    /// 比源码旧了近一小时，设置页的行为与刚修好的代码不符，却看不出是旧二进制，
    /// 白白怀疑了一遍修好的逻辑。
    ///
    /// 构建时间取自运行中二进制文件，所以**能直接暴露陈旧**：
    /// 与源码 mtime 一比就知道该不该重装。
    pub binary_built: String,
    pub model: String,
    pub mode: String,
    pub workspace: String,
    pub branch: String,
    pub session: String,
    pub context_limit: u64,
    pub theme: String,
    pub details: bool,
    pub thinking: bool,
    pub sidebar: bool,
    pub mouse: bool,
    pub clipboard: bool,
    /// 提醒（提示音 / 桌面通知）是否可用
    pub notify: bool,
    /// 是否同时发声
    pub notify_sound: bool,
    /// 是否启用提醒
    pub notify_enabled: bool,
    /// 当前背景
    pub background: String,
    /// 当前 Logo 样式
    pub logo: String,
    /// 是否有自定义背景字符画
    pub custom_background: bool,
    /// 可选模型数量
    pub model_count: usize,
    /// 注册表里的服务商数量（设置页展示）
    pub provider_count: usize,
    /// 会话数（供设置页展示）
    pub session_count: usize,
    pub messages: usize,
    pub files_changed: usize,
}

/// 构造设置分组。
///
/// **只把能改的做成可点击**；改不了的（服务商、模型、上下文上限）
/// 明确写"为什么不能改"，而不是给一个点了没反应的假按钮。
/// 假控件比缺控件更糟 —— 用户会以为功能坏了。
pub fn settings_sections(info: &SettingsInfo) -> Vec<SettingSection> {
    vec![
        SettingSection {
            title: "系统",
            rows: vec![
                SettingRow {
                    label: "版本".into(),
                    value: format!("{}  构建 {}", info.version, info.binary_built),
                    action: None,
                    // 说明为什么这一行能用来判断"我跑的是不是最新构建"
                    readonly_note: "版本为编译期常量；构建时间取自二进制文件，源码更新需重装",
                },
                SettingRow {
                    label: "主题".into(),
                    value: info.theme.clone(),
                    action: Some(SettingAction::ThemePicker),
                    readonly_note: "",
                },
                SettingRow {
                    label: "侧栏".into(),
                    value: if info.sidebar { "展开".into() } else { "收起".to_string() },
                    action: Some(SettingAction::ToggleSidebar),
                    readonly_note: "",
                },
                SettingRow {
                    label: "鼠标".into(),
                    value: if info.mouse { "启用".into() } else { "禁用".to_string() },
                    action: None,
                    readonly_note: "启动参数 NEO_TUI_NO_MOUSE 控制（需重启）",
                },
                SettingRow {
                    label: "剪贴板".into(),
                    value: if info.clipboard { "可用".into() } else { "不可用".to_string() },
                    action: None,
                    readonly_note: "按平台探测；不可用时 /copy 会如实报错",
                },
            ],
        },
        SettingSection {
            title: "模型",
            rows: vec![
                SettingRow {
                    // 这是本轮补上的能力：内核现在持**注册表**，运行时可切换，
                    // 所以这一项从"只读"变成"可选"。注释保留变迁理由，
                    // 免得后人以为早就可以。
                    label: "模型".into(),
                    value: format!("{}（{} 个可选）", info.model, info.model_count),
                    action: Some(SettingAction::ModelPicker),
                    readonly_note: "",
                },
                SettingRow {
                    label: "服务商".into(),
                    // 说明"选了能做什么"：只显示数量会让人以为只能看
                    value: format!("{} 个（选中可改/可删）", info.provider_count),
                    action: Some(SettingAction::ProviderPicker),
                    readonly_note: "",
                },
                SettingRow {
                    label: "新增服务商".into(),
                    value: "填写 base_url / 模型 / 密钥".into(),
                    action: Some(SettingAction::ProviderAdd),
                    readonly_note: "",
                },
                SettingRow {
                    // 删除做成独立一行（而不是藏在表单里的隐藏键）：
                    // 隐藏键没人发现得了，而删除是管理功能的一半。
                    label: "删除服务商".into(),
                    value: "选中后需确认".into(),
                    action: Some(SettingAction::ProviderDelete),
                    readonly_note: "",
                },
                SettingRow {
                    label: "档位".into(),
                    value: info.mode.clone(),
                    action: None,
                    readonly_note: "启动参数 --mode 指定（沙箱 × 审批是内核硬边界）",
                },
                SettingRow {
                    label: "上下文上限".into(),
                    value: if info.context_limit == 0 {
                        "未知".into()
                    } else {
                        format!("{} tokens", info.context_limit)
                    },
                    action: None,
                    readonly_note: "由模型目录决定，本版本未接模型元数据",
                },
            ],
        },
        SettingSection {
            title: "会话",
            rows: vec![
                SettingRow {
                    label: "会话".into(),
                    value: format!("{} 个（用 /sessions 切换）", info.session_count),
                    action: Some(SettingAction::SessionPicker),
                    readonly_note: "",
                },
                SettingRow {
                    label: "会话 ID".into(),
                    value: info.session.clone(),
                    action: None,
                    readonly_note: "用 /sessions 切换会话（本项只读，避免误触切走）",
                },
                SettingRow {
                    label: "工作区".into(),
                    value: info.workspace.clone(),
                    action: None,
                    readonly_note: "启动时的当前目录",
                },
                SettingRow {
                    label: "分支".into(),
                    value: if info.branch.is_empty() { "（非 git 仓库）".into() } else { info.branch.clone() },
                    action: None,
                    readonly_note: "从 .git/HEAD 读取",
                },
                SettingRow {
                    label: "消息数".into(),
                    value: info.messages.to_string(),
                    action: None,
                    readonly_note: "只读统计",
                },
                SettingRow {
                    label: "改动文件".into(),
                    value: info.files_changed.to_string(),
                    action: Some(SettingAction::DiffViewer),
                    readonly_note: "",
                },
                SettingRow {
                    label: "回退对话".into(),
                    value: "回退一轮".into(),
                    action: Some(SettingAction::Rewind),
                    readonly_note: "",
                },
                SettingRow {
                    label: "新对话".into(),
                    // 说明是"新建会话"而不是"清空屏幕"：只清转录会让内核
                    // 仍然记得全部历史，用户以为开了新对话、模型却还在旧上下文里。
                    value: "新建会话".into(),
                    action: Some(SettingAction::NewSessionReal),
                    readonly_note: "",
                },
            ],
        },
        SettingSection {
            title: "显示",
            rows: vec![
                SettingRow {
                    label: "提醒".into(),
                    value: if info.notify_enabled { "开启".into() } else { "关闭".to_string() },
                    action: Some(SettingAction::ToggleNotify),
                    readonly_note: "",
                },
                SettingRow {
                    label: "提醒声音".into(),
                    value: if info.notify_sound { "开".into() } else { "关".to_string() },
                    action: Some(SettingAction::ToggleNotifySound),
                    readonly_note: "",
                },
                SettingRow {
                    label: "背景".into(),
                    value: if info.custom_background {
                        format!("{}（自定义字符画覆盖）", info.background)
                    } else {
                        info.background.clone()
                    },
                    action: Some(SettingAction::NextBackground),
                    readonly_note: "",
                },
                SettingRow {
                    label: "Logo 样式".into(),
                    value: info.logo.clone(),
                    action: Some(SettingAction::NextLogo),
                    readonly_note: "",
                },
                SettingRow {
                    label: "自定义背景".into(),
                    value: if info.custom_background { "已加载".into() } else { "未设置".to_string() },
                    action: None,
                    readonly_note: "用 NEO_TUI_BG_FILE 指定字符画文件（每行即背景一行）",
                },
                SettingRow {
                    label: "提醒后端".into(),
                    value: if info.notify { "系统可用".into() } else { "不可用".to_string() },
                    action: None,
                    readonly_note: "macOS 用 osascript / Linux 用 notify-send；不可用时静默跳过",
                },
                SettingRow {
                    label: "工具输出".into(),
                    value: if info.details { "展开".into() } else { "折叠".to_string() },
                    action: Some(SettingAction::ToggleDetails),
                    readonly_note: "",
                },
                SettingRow {
                    label: "推理过程".into(),
                    value: if info.thinking { "显示".into() } else { "隐藏".to_string() },
                    action: Some(SettingAction::ToggleThinking),
                    readonly_note: "",
                },
            ],
        },
    ]
}

/// 工具输出的展示方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolDisplay {
    /// 是否展开工具输出（`/details` 切换；对齐 opencode 的 tools_details）
    pub expanded: bool,
    /// 是否显示推理过程（`/thinking` 切换）
    pub thinking: bool,
}

impl Default for ToolDisplay {
    fn default() -> Self {
        // 默认折叠：输出常常很长，默认铺开会淹没对话。
        // 但失败时**强制展开**（见渲染处）—— 出错还藏着等于让用户没法自查。
        Self { expanded: false, thinking: false }
    }
}

/// 工具输出的展示上限（行）。超出如实标注，不静默截断。
const TOOL_OUT_LINES: usize = 40;
/// 推理过程的展示上限（行）。
const THINKING_LINES: usize = 12;

fn fact_lines_with(facts: &[Fact], body_cols: usize, disp: ToolDisplay) -> Vec<Vec<Seg>> {
    let inner = body_cols.saturating_sub(4);
    let mut out: Vec<Vec<Seg>> = Vec::new();
    for f in facts {
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
            // 引用解析结果：一行一条摘要（"📄 a.rs 已注入" / "🧩 $x 未找到"）。
            // 正文不在这里渲染 —— 用户要看的是"引用到了没有"，
            // 几百行文件正文会把对话挤没（模型上下文另有注入渠道）。
            Fact::RefsResolved(summary) => {
                for line_text in summary {
                    out.push(vec![(0, format!("  {line_text}"), Tone::Dim)]);
                }
                out.push(Vec::new());
            }
            // 指令来源（用户需要知道会话受哪些约定约束；截断必须显式告警）
            Fact::InstructionsLoaded { sources, truncated } => {
                let text = format!(
                    "  ⚑ 项目指令：{} 个文件{}",
                    sources.len(),
                    if *truncated { "（已按 32 KiB 截断）" } else { "" }
                );
                out.push(vec![(0, text, Tone::Dim)]);
                out.push(Vec::new());
            }
            Fact::AssistantThought(text) => {
                // 推理默认**不显示**（`/thinking` 打开）：它常常很长且是过程性
                // 内容，默认铺开会把答复挤下去。但必须可选可见 —— 排查模型
                // 为什么做错时，看推理往往比看答复有用。
                if !disp.thinking {
                    continue;
                }
                out.push(vec![(2, "⋯ 思考".to_string(), Tone::Border)]);
                let all: Vec<&str> = text.lines().collect();
                for l in all.iter().take(THINKING_LINES) {
                    for w in width::wrap_to_width(l, inner.saturating_sub(6)) {
                        out.push(vec![(4, "│ ".to_string(), Tone::Border), (6, w, Tone::Muted)]);
                    }
                }
                if all.len() > THINKING_LINES {
                    out.push(vec![(
                        6,
                        format!("… 另有 {} 行思考未显示", all.len() - THINKING_LINES),
                        Tone::Border,
                    )]);
                }
            }

            Fact::AssistantSaid(text) => {
                // 助手回复按 Markdown 渲染：代码块高亮、行内代码、标题、列表。
                // inner 已扣掉侧栏占用（render 把正文写入裁到侧栏左侧）。
                out.extend(markdown::render(text, inner));
                out.push(Vec::new());
            }
            Fact::ToolFinished { name, exit_code, stdout, stderr, truncated } => {
                let ok = *exit_code == 0;
                let (icon, tone) =
                    if ok { ("✓", Tone::Success) } else { ("✗", Tone::Error) };
                let after = 4 + width::display_width(name);
                // 折叠时给出"有多少输出"的提示，否则用户不知道藏了东西
                let lines = stdout.lines().count() + stderr.lines().count();
                let suffix = if ok && !disp.expanded {
                    if lines > 0 {
                        format!("exit {exit_code} · {lines} 行输出（/details 展开）")
                    } else {
                        format!("exit {exit_code}")
                    }
                } else {
                    format!("exit {exit_code}")
                };
                out.push(vec![
                    (2, format!("{icon} "), tone),
                    (4, name.clone(), Tone::Text),
                    (after + 1, suffix, Tone::Muted),
                ]);
                // 展示输出：展开时全给，未展开时**失败也强制给** ——
                // 出错还把原因藏起来，用户只能靠猜。
                let show = disp.expanded || !ok;
                if show {
                    let mut shown = 0usize;
                    for (text, t) in [(stdout.as_str(), Tone::Muted), (stderr.as_str(), Tone::Error)] {
                        if text.trim().is_empty() {
                            continue;
                        }
                        for l in text.lines() {
                            if shown >= TOOL_OUT_LINES {
                                break;
                            }
                            for w in width::wrap_to_width(l, inner.saturating_sub(6)) {
                                out.push(vec![(4, "│ ".to_string(), Tone::Border), (6, w, t)]);
                                shown += 1;
                            }
                        }
                    }
                    let total = stdout.lines().count() + stderr.lines().count();
                    if total > shown || *truncated {
                        let note = if *truncated {
                            format!("… 输出已被截断，仅显示前 {shown} 行")
                        } else {
                            format!("… 另有 {} 行未显示（/details 展开）", total - shown)
                        };
                        out.push(vec![(6, note, Tone::Border)]);
                    }
                }
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
            Fact::ContextCompacted { removed_messages } => {
                out.push(vec![(
                    2,
                    format!("⇩ 上下文已压缩（{removed_messages} 条 → 摘要）"),
                    Tone::Info,
                )]);
            }
            Fact::Rewound { turns, removed_messages, files_kept } => {
                // 必须同时说明"文件没回退" —— 让人以为文件也回去了最危险
                out.push(vec![(
                    2,
                    format!("↩ 已回退 {turns} 轮（{removed_messages} 条消息）"),
                    Tone::Info,
                )]);
                if *files_kept > 0 {
                    out.push(vec![(
                        4,
                        format!("注意：磁盘上 {files_kept} 个文件的改动未被撤销"),
                        Tone::Warning,
                    )]);
                }
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
            Fact::ModelSwitched { model, context_limit } => {
                // 模型切换是用户会关心的状态变化 —— 明确告知
                let tail = if *context_limit == 0 {
                    String::new()
                } else {
                    format!("（上下文 {context_limit}）")
                };
                out.push(vec![(2, format!("⇄ 模型已切换为 {model}{tail}"), Tone::Info)]);
            }
        }
    }
    out
}

/// 转录渲染后的行数（滚动边界用）。
///
/// 与 `fact_lines` 的换行结果保持一致：用同一个 Markdown 渲染与宽度预算，
/// 否则滚动上限会与实际可滚范围不符（表现：滚不到底或滚出空白）。
fn transcript_line_count(events: &[EventMsg], body_cols: usize) -> usize {
    // 与渲染同源：直接数 fact_lines 的行数
    fact_lines(&facts_of(events), body_cols).len()
}

/// 把事实渲染成可搜索的纯文本行。
///
/// **直接复用 `fact_lines`**：搜索命中行号必须与渲染行号一致。
/// 也不能用 `format!("{f:?}")` —— 那会把枚举名纳入匹配，
/// 用户搜 "Assistant" 会命中每一行（屏幕上根本没这个词）。
fn searchable_lines(facts: &[Fact], body_cols: usize) -> Vec<String> {
    // 直接复用 fact_lines：搜索命中行号必须与渲染行号一致
    fact_lines(facts, body_cols)
        .into_iter()
        .map(|segs| segs.into_iter().map(|(_, t, _)| t).collect::<String>())
        .collect()
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

/// 前 `visible` 项会画出多少个分组标题（与 draw_popup 的判定一致）。
fn group_headers(items: &[popup::Item], visible: usize) -> usize {
    let mut n = 0;
    let mut prev: Option<&str> = None;
    for it in items.iter().take(visible) {
        if it.category.is_empty() {
            continue;
        }
        if prev != Some(it.category) {
            n += 1;
            prev = Some(it.category);
        }
    }
    n
}

/// 弹窗内内容的行上限（不含边框），防止分组标题把内容挤出框外。
fn bottom_limit(top: usize, height: usize) -> usize {
    top + height - 1
}

/// 弹窗上方能容纳的总行数（含边框）。0 表示连边框都放不下。
fn available_rows(avail: usize) -> usize {
    if avail < 3 {
        0
    } else {
        avail
    }
}

/// 鼠标点中某个区域后要执行的动作。
#[derive(Debug, Clone, PartialEq, Eq)]
enum MouseAction {
    /// 在转录区滚动（true = 向上看更早的内容）
    ScrollTranscript(bool),
    /// 选中弹窗第 N 项
    SelectPopup(usize),
    /// 点到输入框
    FocusInput,
    /// 点到侧栏（切换显隐）
    ToggleSidebar,
    /// 点到设置页第 N 个可操作项（N 与 `settings_cursor` 同一空间）
    SettingsRow(usize),
}

/// 从当前事件流里的 PatchPreview 打开 diff 查看器。
///
/// 汇总**所有**待审批/已发生的改动（按文件拼接），而不是只看最后一条 ——
/// 用户按 `/diff` 想看的是"这次会话改了什么"。
fn open_diff_viewer(events: &[EventMsg], slot: &mut Option<diffview::Viewer>) {
    let mut text = String::new();
    let facts = facts_of(events);
    for f in &facts {
        if let Fact::PatchPreview { diff, .. } = f {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(diff);
            // 多文件之间需要换行分隔，否则两个 `+++` 会连在一起
            if !diff.ends_with('\n') {
                text.push('\n');
            }
        }
    }
    let parsed = diffview::parse(&text);
    *slot = Some(diffview::Viewer::new(parsed));
}

/// 提交一次对话回退并把结果并入事件流。
///
/// 抽成函数是因为三个入口（弹窗 Enter / `/命令` / Tab 接受）都要用它；
/// 各写一遍必然漂移（例如某处忘了把命中事件并进 events）。
fn do_rewind<F>(submit: &mut F, events: &mut Vec<EventMsg>, status: &mut String)
where
    F: FnMut(neo_protocol::Op) -> Result<Vec<EventMsg>, String>,
{
    match submit(neo_protocol::Op::Rewind { turns: 1 }) {
        Ok(produced) => {
            events.extend(produced);
            *status = "已回退一轮对话（文件改动未撤销；/diff 可查看）".to_string();
        }
        Err(e) => *status = format!("回退失败：{e}"),
    }
}

/// 处理会话类 Effect（切换 / 新建 / 删除）。
///
/// 操作由调用方的 `SessionControl` 执行 —— 宿主不重建内核、不读日志，
/// 只把返回的事件流换成新的 Facts。返回给状态栏的文案。
fn handle_session_effect(
    eff: &Effect,
    sessions: &mut dyn SessionControl,
    events: &mut Vec<EventMsg>,
    view: &mut view::View,
) -> Option<String> {
    match eff {
        Effect::SwitchSession(id) => {
            match sessions.switch(id) {
                Ok(history) => {
                    // 用重建出的历史**替换**当前转录，并把视图滚到底
                    *events = history;
                    view.to_bottom();
                    Some(format!("已切换到会话 {id}"))
                }
                Err(e) => Some(format!("切换失败：{e}")),
            }
        }
        Effect::NewSessionReal => match sessions.create() {
            Ok(id) => {
                // 新会话 = 空转录；旧会话已在磁盘上，可 /sessions 切回
                events.clear();
                view.to_bottom();
                Some(format!("已新建会话 {id}（旧会话保留，/sessions 可切回）"))
            }
            Err(e) => Some(format!("新建会话失败：{e}")),
        },
        Effect::DeleteSession(id) => match sessions.delete(id) {
            Ok(true) => Some(format!("已删除会话 {id}")),
            Ok(false) => Some(format!("会话 {id} 不存在（可能已被删除）")),
            Err(e) => Some(format!("删除失败：{e}")),
        },
        _ => None,
    }
}

/// 处理外观与模型这类"纯界面状态"的 Effect。
///
/// 抽成一个函数是为了让四处弹窗分支都用同一套逻辑 —— 之前每处各写一段
/// match，加一个变体就要改四处（而且很容易漏掉某一处）。
///
/// 返回 `Some(文案)` 表示已处理；`None` 表示"不是这里管的"（调用方应忽略）。
fn handle_ui_effect(
    eff: &Effect,
    ap: &mut appearance::Appearance,
    popup_state: &mut Option<popup::Popup>,
    about: &About,
    sessions: &dyn SessionControl,
) -> Option<String> {
    // 外观类
    if let Some(m) = apply_appearance(ap, eff) {
        return Some(m);
    }
    match eff {
        Effect::BackgroundPicker => {
            let mut tp = popup::Popup::new(popup::Kind::Background, "");
            tp.set_items(popup::background_items(""), false);
            *popup_state = Some(tp);
            Some("背景 · ↑↓ 选择，回车应用".into())
        }
        Effect::LogoPicker => {
            let mut tp = popup::Popup::new(popup::Kind::Logo, "");
            tp.set_items(popup::logo_items(""), false);
            *popup_state = Some(tp);
            Some("Logo 样式 · ↑↓ 选择，回车应用".into())
        }
        Effect::ModelPicker => {
            let mut tp = popup::Popup::new(popup::Kind::Models, "");
            tp.set_items(popup::model_items(&about.models, &about.current_model), false);
            *popup_state = Some(tp);
            Some("模型 · ↑↓ 选择，回车切换".into())
        }
        Effect::SessionPicker => {
            let list = sessions.list();
            let cur = sessions.current();
            let mut tp = popup::Popup::new(popup::Kind::Sessions, "");
            tp.set_items(popup::session_items(&list, &cur), false);
            *popup_state = Some(tp);
            Some(format!("会话 · {} 个可选，↑↓ 选择", list.len()))
        }
        _ => None,
    }
}

/// 应用外观变更并落盘。返回给状态栏的文案。
fn apply_appearance(ap: &mut appearance::Appearance, eff: &Effect) -> Option<String> {
    match eff {
        Effect::NextBackground => {
            ap.background = ap.background.next();
            appearance::save_preference(*ap);
            Some(format!("背景：{}", ap.background.as_str()))
        }
        Effect::NextLogo => {
            ap.logo = ap.logo.next();
            appearance::save_preference(*ap);
            Some(format!("Logo：{}", ap.logo.as_str()))
        }
        Effect::SetBackground(b) => {
            ap.background = *b;
            appearance::save_preference(*ap);
            Some(format!("背景：{}", b.as_str()))
        }
        Effect::SetLogo(l) => {
            ap.logo = *l;
            appearance::save_preference(*ap);
            Some(format!("Logo：{}", l.as_str()))
        }
        _ => None,
    }
}

/// 运行中二进制文件的构建时间（mtime），形如 `2026-09-11 17:45`。
///
/// 取不到就返回 `"未知"` —— 不编一个假时间。这一项的存在意义是让
/// **陈旧构建可见**：`version` 是编译期常量，改代码重编译也不会变，
/// 于是"我跑的到底是哪一版"从界面上判断不出来（真实踩过：
/// PATH 里的 neo 比源码旧近一小时，行为与刚修好的代码不符却查不出原因）。
fn binary_build_time() -> String {
    use std::time::UNIX_EPOCH;
    let Ok(exe) = std::env::current_exe() else { return "未知".into() };
    let Ok(meta) = std::fs::metadata(&exe) else { return "未知".into() };
    let Ok(modified) = meta.modified() else { return "未知".into() };
    let Ok(d) = modified.duration_since(UNIX_EPOCH) else { return "未知".into() };
    format_unix_utc_minute(d.as_secs())
}

/// 把 Unix 秒格式化成 `YYYY-MM-DD HH:MM`（UTC，零依赖实现）。
///
/// 为什么不用 chrono：本项目刻意零外部依赖（"调试链要浅"）。
/// 只需要"能比较新旧"这一点精度，不必引入日历库。
fn format_unix_utc_minute(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, mi) = (rem / 3600, (rem % 3600) / 60);
    // 民用历：从 1970-01-01 起逐月推进
    let mut y = 1970i64;
    let mut d = days as i64;
    loop {
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let ylen = if leap { 366 } else { 365 };
        if d < ylen { break; }
        d -= ylen;
        y += 1;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let ml = [31, if leap {29} else {28}, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0usize;
    while d >= ml[m] { d -= ml[m]; m += 1; }
    format!("{y:04}-{:02}-{:02} {h:02}:{mi:02}", m + 1, d + 1)
}

/// 组装设置视图的内容（**不动光标位置**）。
///
/// 打开时与"动作后刷新"时都调它。光标不在这里复位是刻意的：
/// 之前它在每次刷新时被清零，用户在第 9 行按回车、光标立刻跳回第 1 行，
/// 画面像是"闪一下没反应"——实际动作执行了，只是选中位置被重置。
/// 复位是"打开"这一动作的语义，交给打开方显式做。
#[allow(clippy::too_many_arguments)]
fn open_settings(
    slot: &mut Option<Vec<SettingSection>>,
    about: &About,
    events: &[EventMsg],
    display: &ToolDisplay,
    sidebar: bool,
    mouse: bool,
    clipboard: &dyn neo_platform::Clipboard,
    notify_backend: &dyn neo_platform::Notify,
    notify_enabled: bool,
    notify_sound: bool,
    theme_name: theme::ThemeName,
    appearance: appearance::Appearance,
    has_custom_bg: bool,
    provider_count: usize,
    sessions: &dyn SessionControl,
) {
    let facts = facts_of(events);
    let files_changed = facts
        .iter()
        .rev()
        .find_map(|f| match f {
            Fact::FilesChanged(files) => Some(files.len()),
            _ => None,
        })
        .unwrap_or(0);
    let info = SettingsInfo {
        // 提醒的三项来自真实后端与当前开关，不是硬编码 ——
        // 硬编码会让设置页显示"开启"而实际没开。
        notify: notify_backend.available(),
        notify_sound,
        notify_enabled,
        model_count: about.models.len(),
        provider_count,
        session_count: sessions.list().len(),
        background: appearance.background.as_str().to_string(),
        logo: appearance.logo.as_str().to_string(),
        custom_background: has_custom_bg,
        version: about.version.clone(),
        binary_built: binary_build_time(),
        // 用 current_model（实时）而不是 model（启动快照）：切换后必须显示新模型，
        // 否则"切成功了界面还说旧的"—— 与 session 同一个坑。
        model: about.current_model.clone(),
        mode: about.mode_short.clone(),
        workspace: about.workspace.clone(),
        branch: about.branch.clone(),
        session: sessions.current(),
        context_limit: about.context_limit,
        theme: theme_name.as_str().to_string(),
        details: display.expanded,
        thinking: display.thinking,
        sidebar,
        mouse,
        clipboard: clipboard.available(),
        messages: events.len(),
        files_changed,
    };
    *slot = Some(settings_sections(&info));
}

/// 打开设置视图（并把光标复位到第一项）。
///
/// "打开"才复位光标；"刷新"（动作执行后）调 [`open_settings`] 保留位置。
fn open_settings_fresh(
    slot: &mut Option<Vec<SettingSection>>,
    cursor: &mut usize,
    about: &About,
    events: &[EventMsg],
    display: &ToolDisplay,
    sidebar: bool,
    mouse: bool,
    clipboard: &dyn neo_platform::Clipboard,
    notify_backend: &dyn neo_platform::Notify,
    notify_enabled: bool,
    notify_sound: bool,
    theme_name: theme::ThemeName,
    appearance: appearance::Appearance,
    has_custom_bg: bool,
    provider_count: usize,
    sessions: &dyn SessionControl,
) {
    *cursor = 0;
    open_settings(
        slot, about, events, display, sidebar, mouse, clipboard, notify_backend,
        notify_enabled, notify_sound, theme_name, appearance, has_custom_bg,
        provider_count, sessions,
    );
}

/// 构造服务商表单。`existing = Some(...)` 时是编辑（密钥留空表示不修改）。
fn provider_form(name: Option<&str>, providers: &dyn ProviderControl) -> SettingsForm {
    let mk = |label: &str, val: &str, secret: bool| FormField {
        label: label.to_string(),
        editor: editor::Editor::from_text(val),
        secret,
    };
    let (n, base, model, desc, limit) = match name {
        Some(n) => {
            let (b, m, d, l) = providers.get(n).unwrap_or_default();
            (n.to_string(), b, m, d, l)
        }
        None => (String::new(), String::new(), String::new(), String::new(), 0),
    };
    let key_label = if name.is_some() { "密钥(留空不改)" } else { "密钥" };
    let mut form = SettingsForm::new(
        if name.is_some() { format!("编辑服务商：{n}") } else { "新增服务商".to_string() },
        vec![
            mk("名称", &n, false),
            mk("base_url", &base, false),
            mk("模型", &model, false),
            FormField { label: key_label.to_string(), editor: editor::Editor::new(), secret: true },
            mk("描述", &desc, false),
            mk("上下文", &if limit == 0 { String::new() } else { limit.to_string() }, false),
        ],
        Some(3),
    );
    form.editing = name.map(String::from);
    form
}

/// 把表单提交给契据。返回给状态栏的文案。
fn commit_provider_form(form: &SettingsForm, providers: &mut dyn ProviderControl) -> String {
    let name = form.trimmed(0);
    let base_url = form.trimmed(1);
    let model = form.trimmed(2);
    let api_key = form.trimmed(3);
    let desc = form.trimmed(4);
    // 上下文上限：留空 = 0（未知）。非数字**如实报错**，不要静默当 0 ——
    // 静默会让用户以为自己填对了。
    let limit_txt = form.trimmed(5);
    let limit = if limit_txt.is_empty() {
        0
    } else {
        match limit_txt.parse::<u64>() {
            Ok(v) => v,
            Err(_) => return format!("上下文上限不是数字：{limit_txt}"),
        }
    };
    match providers.upsert(
        &name,
        &base_url,
        &model,
        &api_key,
        &desc,
        limit,
        form.editing.as_deref(),
    ) {
        Ok(()) => {
            let verb = if form.editing.is_some() { "已更新" } else { "已新增" };
            // **立刻生效**，所以不再写"重启后生效" —— 那句话既没说清重启什么，
            // 也让用户以为现在不能用。若该服务商还没配密钥，如实说明它为何
            // 不在模型列表里（否则用户会以为新增失败）。
            let has_key = providers.list().iter().any(|(n, _, k)| n == &name && *k);
            if has_key {
                format!("{verb}服务商 {name}（已可切换，见「模型」）")
            } else {
                format!("{verb}服务商 {name}（尚无密钥，填了才会出现在「模型」里）")
            }
        }
        Err(e) => format!("保存失败：{e}"),
    }
}

/// 设置视图里"可操作行"的扁平序号 → (节, 行)。
fn settings_actionable(sections: &[SettingSection]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (si, sec) in sections.iter().enumerate() {
        for (ri, r) in sec.rows.iter().enumerate() {
            if r.action.is_some() {
                out.push((si, ri));
            }
        }
    }
    out
}

/// 复制最近一条回复到剪贴板，返回给状态栏的文案。
///
/// **成功与失败都要如实说**：剪贴板 SPI 的 `copy` 会真报错（找不到命令、
/// 命令非 0 退出）。这里把它转成用户能懂的一句话，而不是笼统的"已复制"。
fn do_copy(events: &[EventMsg], clipboard: &dyn neo_platform::Clipboard) -> String {
    let facts = facts_of(events);
    let Some(text) = last_reply(&facts) else {
        return "没有可复制的回复".to_string();
    };
    match clipboard.copy(&text) {
        Ok(()) => format!("已复制 {} 个字符到剪贴板", text.chars().count()),
        Err(e) => format!("复制失败：{e}"),
    }
}

/// 找出**新出现**的、值得提醒的信号。
///
/// 只提醒三类（对齐 opencode 的 attention）：完成、出错、需审批。
/// `seen` 用来去重 —— 否则每帧重扫 facts 会重复响。
fn attention_signals(
    facts: &[Fact],
    seen: &std::collections::HashSet<String>,
) -> Vec<(String, neo_platform::Attention, String)> {
    let mut out = Vec::new();
    for f in facts {
        let (key, kind, detail) = match f {
            Fact::TurnFinished { input_tokens, output_tokens } => (
                format!("done:{input_tokens}:{output_tokens}"),
                neo_platform::Attention::TurnComplete,
                format!("完成（{input_tokens} in / {output_tokens} out）"),
            ),
            Fact::Failed(msg) => (
                format!("err:{}", msg.chars().take(64).collect::<String>()),
                neo_platform::Attention::Error,
                msg.chars().take(120).collect(),
            ),
            Fact::ApprovalNeeded { detail } => (
                format!("ask:{}", detail.chars().take(64).collect::<String>()),
                neo_platform::Attention::ApprovalNeeded,
                format!("需要审批：{detail}"),
            ),
            _ => continue,
        };
        if !seen.contains(&key) {
            out.push((key, kind, detail));
        }
    }
    out
}


/// 取最近一条助手回复（供 `/copy`）。
///
/// 只看 `AssistantSaid`（**不含推理**）—— 用户要复制的是答复，不是思考过程。
fn last_reply(facts: &[Fact]) -> Option<String> {
    facts.iter().rev().find_map(|f| match f {
        Fact::AssistantSaid(t) if !t.trim().is_empty() => Some(t.clone()),
        _ => None,
    })
}

/// 正文可用列数（与 `Screen::body_cols` 同一判据）。
fn self_body_cols(cols: usize, sidebar: bool) -> usize {
    if sidebar && cols >= SIDEBAR_MIN_COLS {
        cols.saturating_sub(SIDEBAR_COLS)
    } else {
        cols
    }
}

/// 把鼠标事件映射成动作。
///
/// 命中优先级：**弹窗 > 输入框 > 侧栏 > 转录**。
/// 弹窗在最上层（它盖住正文），所以优先；
/// 侧栏在转录右侧、输入框之外，故排在输入框之后。
///
/// 只处理"按下"，忽略释放：终端会把一次点击拆成按下+释放两个事件，
/// 两边都处理会导致动作执行两次。
fn hit_test(
    r: &Regions,
    ev: &mouse::MouseEvent,
    popup: Option<&popup::Popup>,
    sidebar_open: bool,
) -> Option<MouseAction> {
    // 滚轮没有按下/释放之分，先处理（且不要求在某个区域内才生效 ——
    // 用户在正文任意处滚轮都应滚动转录）
    match ev.button {
        mouse::Button::WheelUp => return Some(MouseAction::ScrollTranscript(true)),
        mouse::Button::WheelDown => return Some(MouseAction::ScrollTranscript(false)),
        _ => {}
    }
    if !ev.pressed {
        return None;
    }

    // 设置页：点在可操作行上 → 执行该项（与 Enter 同一分支）。
    // 排在弹窗之前判断也无妨：设置视图是独占的，不会与弹窗同时出现。
    //
    // `+ 1` 是必须的：`settings_rows` 存的是**网格行号（0 基）**，
    // 而 `ev.y` 来自 SGR 鼠标序列，是**终端行号（1 基）**。
    // 漏掉这个转换，点击会整体错一行（或永远不命中）——
    // 与 `popup_items` 用同一约定，不要在这里另立一套。
    for (y, idx) in &r.settings_rows {
        if ev.y == *y + 1 {
            return Some(MouseAction::SettingsRow(*idx));
        }
    }

    // 侧栏把手（收起状态下唯一能展开的鼠标入口）
    if let Some((gx, gy)) = r.sidebar_grip {
        if ev.x == gx && ev.y == gy {
            return Some(MouseAction::ToggleSidebar);
        }
    }

    // 弹窗候选
    if popup.is_some() {
        for ((y, x0, x1), index) in &r.popup_items {
            if ev.y == *y + 1 && ev.x > *x0 && ev.x <= *x1 {
                return Some(MouseAction::SelectPopup(*index));
            }
        }
        // 点在弹窗范围内但不在某一行上：不做事（避免误点关闭）
        return None;
    }

    // 输入框（用 1 基坐标）
    if let Some((l, rr, top, bottom)) = r.input_box {
        if ev.x > l && ev.x <= rr && ev.y > top && ev.y <= bottom {
            return Some(MouseAction::FocusInput);
        }
    }

    // 侧栏
    if let Some((x0, x1, y0, y1)) = r.sidebar {
        if ev.x > x0 && ev.x <= x1 && ev.y > y0 && ev.y <= y1 {
            return Some(MouseAction::ToggleSidebar);
        }
    }

    // 转录区
    if let Some((top, bottom)) = r.transcript {
        if ev.y > top && ev.y <= bottom {
            return Some(MouseAction::ScrollTranscript(false));
        }
    }
    let _ = sidebar_open;
    None
}

/// 状态栏文案：审批挂起时明确告诉用户该敲什么，否则是常规就绪提示。
fn idle_or_approval(outstanding: &Option<String>) -> String {
    match outstanding {
        Some(_) => "待审批 · y 批准 / a 总是允许这类 / n 拒绝".to_string(),
        // 空闲时不再堆一串按键说明 —— 输入框有占位提示、下方有提示行
        None => "就绪".to_string(),
    }
}

/// 运行 TUI，直到用户退出。
///
/// `submit` 由调用方注入（通常是 `kernel.submit`），这样本 crate
/// **不依赖 neo-orchestration / L3 之上的任何东西**，只依赖契据。
pub fn run<F>(
    mut about: About,
    trust_workspace: Option<std::path::PathBuf>,
    mut theme_name: theme::ThemeName,
    sessions: &mut dyn SessionControl,
    providers: &mut dyn ProviderControl,
    mut submit: F,
) -> std::io::Result<()>
where
    F: FnMut(neo_protocol::Op) -> Result<Vec<EventMsg>, String>,
{
    // 可重入：`ctrl+z` 挂起时要先还原终端、恢复后再进入
    let mut raw = RawMode::enter().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    // 进备用屏：全程在第二块屏幕上画，退出时终端整块还原
    let mut alt = AltScreen::enter();
    // 鼠标上报（NEO_TUI_NO_MOUSE=1 可关，给"只想用键盘"或终端不支持的用户）
    let mouse_on = std::env::var_os("NEO_TUI_NO_MOUSE").is_none();
    let mut mouse = MouseMode::enter(mouse_on);

    // panic 时也还原终端，否则用户的终端会被留在原始模式
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = set_stty("sane");
        // 鼠标上报也必须关掉：否则 panic 后 shell 里鼠标会乱跳
        MouseMode::leave_now();
        // 必须退出备用屏，否则 panic 后用户的终端会一直停在我们这里
        AltScreen::leave_now();
        println!("\r\n[tui] 发生 panic，终端已还原");
        previous_hook(info);
    }));

    let mut stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    let mut input = editor::Editor::new();
    // 给"不需要输入框内容"的帧（信息屏/过渡帧/信任页）复用一个空编辑器，
    // 避免每处都构造一份
    let empty_input = editor::Editor::new();
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
    // 审批对话框内容（模态）。`None` = 无待审批。
    let mut approval: Option<ApprovalPrompt> = None;
    // 弹窗（@ / / / 主题 / 面板）
    let mut popup_state: Option<popup::Popup> = None;
    // 只读信息屏（/help、/keys）：显示到用户按任意键
    let mut info_screen: Option<String> = None;
    // 由命令/弹窗设置：请求退出主循环
    let mut should_quit = false;
    // 侧栏开关（`ctrl+b`）。窄终端下渲染时会强制隐藏。
    let mut sidebar_open = true;
    // 转录滚动/搜索状态
    let mut view_state = view::View::new();
    // 全屏 diff 查看器（`/diff` 或审批时按 d 打开）
    let mut diff_viewer: Option<diffview::Viewer> = None;
    // which-key 提示（`ctrl+/`）：任意键关闭
    let mut whichkey_groups: Option<Vec<whichkey::Group>> = None;
    // 工具输出 / 推理的显示方式（`/details` `/thinking` 切换）
    let mut display = ToolDisplay::default();
    // 外观：背景纹理 + Logo 样式（`/background` `/logo` 切换，落盘记忆）
    let mut current_appearance = appearance::load_preference();
    // 自定义背景字符画（`NEO_TUI_BG_FILE` 指到文件时优先）
    let custom_bg = std::env::var_os("NEO_TUI_BG_FILE")
        .map(std::path::PathBuf::from)
        .and_then(|p| appearance::load_custom_background(&p, 200, 400));
    // 设置视图（`ctrl+p` → 设置，或 `/settings`）
    let mut settings_state: Option<Vec<SettingSection>> = None;
    let mut settings_cursor: usize = 0;
    // 设置页内的内联选择器（在该行下方展开候选）
    let mut settings_picker: Option<SettingsPicker> = None;
    // 选择器里确认的一项：在本帧结束时统一执行（与 popup 的 ItemAction 复用同一处理）
    let mut picker_apply: Option<popup::Item> = None;
    // 服务商表单（新增/编辑）。Some = 接管设置页输入。
    let mut settings_form: Option<SettingsForm> = None;
    // 注册表里的服务商数量（设置页展示）—— 来自契据，宿主不自己读配置
    let provider_count = providers.list().len();
    // 表单保存后要落盘的动作（在表单关闭后执行，避免借用冲突）
    let mut form_commit: Option<SettingsForm> = None;
    // 待删除的服务商名（表单里按 ctrl+d）。**执行前要确认**：
    // 删除会连带密钥一起丢掉，误触的代价太大。
    let mut pending_delete: Option<String> = None;
    // 确认对话：Some((问题, 是→待执行的动作))
    let mut confirm_delete: Option<String> = None;
    // 提醒：默认关闭（对齐 opencode 的 attention.enabled 默认 false）——
    // 没人喜欢工具自己响。要就显式开：/settings 或 NEO_TUI_NOTIFY=1。
    let mut notify_enabled = std::env::var_os("NEO_TUI_NOTIFY").is_some();
    let mut notify_sound = std::env::var_os("NEO_TUI_NOTIFY_SILENT").is_none();
    let notify_backend: Box<dyn neo_platform::Notify> =
        if std::env::var_os("NEO_TUI_NO_NOTIFY").is_some() {
            Box::new(neo_platform::NoopNotify::new("已通过 NEO_TUI_NO_NOTIFY 禁用"))
        } else {
            Box::new(neo_platform::SystemNotify::new(notify_sound))
        };
    // 已提醒过的信号，避免同一个事件重复响（每帧都会重新扫 facts）
    let mut notified: std::collections::HashSet<String> = std::collections::HashSet::new();
    // 剪贴板：按平台选后端；不可用时退化为 noop（`/copy` 会如实报错）
    let clipboard: Box<dyn neo_platform::Clipboard> =
        if std::env::var_os("NEO_TUI_NO_CLIPBOARD").is_some() {
            Box::new(neo_platform::NoopClipboard::new("已通过 NEO_TUI_NO_CLIPBOARD 禁用"))
        } else {
            Box::new(neo_platform::SystemClipboard::new())
        };
    // 最近一次渲染记录的命中区域。**必须跨迭代保留** ——
    // 只有 dirty 时才重绘，若把它声明在循环内，鼠标事件到达时
    // 区域是空的，命中测试永远失败（点击/滚动全部无效）。
    let mut last_regions = Regions::default();
    // 搜索输入模式：Some = 正在输入查询词（Enter 确认，Esc 取消）
    let mut searching = false;

    // ── 首次进入某工作区：先要一次知情同意 ──────────────────────────
    //
    // 沙箱是硬边界（最多能做什么），信任是知情同意（这个目录是不是你让我动的）。
    // 二者互补：沙箱挡不住"信任错了目录"，信任挡不住"恶意代码越权"。
    if let Some(ws) = trust_workspace {
        let mut tp = TrustPrompt::default();
        // 与主循环同样的 dirty 策略：带超时读键让循环每 0.1 秒醒一次，
        // 若每次都重绘就是 10Hz 空转（终端持续刷屏 + 白耗 CPU）。
        let mut trust_dirty = true;
        loop {
            let (cols, rows) = terminal_size();
            if !trust_dirty {
                // 没事可做：只等输入（超时后回到这里再检查尺寸）
                match read_key_timeout(&mut stdin, 1) {
                    None => {
                        if terminal_size() == (cols, rows) {
                            continue;
                        }
                        trust_dirty = true;
                    }
                    Some(k) => {
                        match handle_trust_key(k, &mut tp, &ws) {
                            TrustOutcome::Continue => {
                                trust_dirty = true;
                                continue;
                            }
                            TrustOutcome::Accepted => break,
                            TrustOutcome::Quit => {
                                raw.restore();
                                alt.leave();
                                return Ok(());
                            }
                        }
                    }
                }
            }
            if !trust_dirty {
                continue;
            }
            let screen = Screen {
                cols,
                rows,
                facts: &[],
                input: &empty_input,
                status: "",
                awaiting_input: false,
                approval: None,
                show_cursor: false,
                about: Some(&about),
                trust: Some(&tp),
                theme: theme_name,
                popup: None,
                preformatted: None,
                sidebar: false,
                view: None,
                diff_viewer: None,
                whichkey: None,
                display: ToolDisplay::default(),
                settings: None,
                settings_cursor: 0,
            settings_form: None, settings_confirm: None,
                settings_picker: None,
                appearance: appearance::Appearance::default(),
                custom_background: None,
            };
            write!(stdout, "{}", screen.render())?;
            stdout.flush()?;

            trust_dirty = false;
            let Some(k) = read_key_timeout(&mut stdin, 1) else {
                continue; // 超时：回顶部重新检查尺寸
            };
            match handle_trust_key(k, &mut tp, &ws) {
                TrustOutcome::Continue => trust_dirty = true,
                TrustOutcome::Accepted => break,
                TrustOutcome::Quit => {
                    raw.restore();
                    mouse.leave();
                    alt.leave();
                    return Ok(());
                }
            }
        }
    }

    // 是否需要重绘。带超时的读键会让循环每 0.1 秒醒一次；若每次都重绘，
    // 就是 10Hz 空转（终端抖屏 + 白耗 CPU）。只在"输入被处理"或
    // "窗口尺寸变化"时置位。
    let mut dirty = true;

    loop {
        let (cols, rows) = terminal_size();

        // ── 设置视图：独占输入 ──────────────────────────────────────
        if settings_state.is_some() {
            // 取一份**拥有所有权**的分节快照：本分支里会重新赋值 settings_state
            // （关闭、或动作后刷新），持着 `as_ref()` 的借用就改不动它。
            // 快照只有几十个小结构体，每帧一次的开销远小于一次终端重绘。
            let sections = settings_state.clone().unwrap_or_default();
            let sections = &sections;
            let screen = Screen {
                cols,
                rows,
                facts: &[],
                input: &empty_input,
                // 传真实状态：设置页的动作很多只改状态文案，
                // 硬编码 "" 会让这些动作"做了但看不见"（见 draw_settings 注释）。
                status: &status,
                awaiting_input: false,
                approval: None,
                show_cursor: false,
                about: Some(&about),
                trust: None,
                theme: theme_name,
                popup: None,
                preformatted: None,
                sidebar: false,
                view: None,
                diff_viewer: None,
                whichkey: None,
                display,
                settings: Some(sections),
                settings_cursor,
                settings_picker: settings_picker.as_ref(),
                settings_form: settings_form.as_ref(),
                settings_confirm: confirm_delete.as_deref(),
                appearance: current_appearance,
                custom_background: custom_bg.as_ref(),
            };
            let (rendered, sregions) = screen.render_with_regions();
            write!(stdout, "{rendered}")?;
            stdout.flush()?;

            let slots = settings_actionable(sections);
            // 内联选择器里确认的一项：复用 popup 的 ItemAction 处理，
            // 不另写一套 —— 主题/模型/会话的行为与 `/theme`、`/model`、`/sessions`
            // 必须完全一致，复制一份必然漂移。
            if let Some(item) = picker_apply.take() {
                let eff = apply_popup_item(
                    &item,
                    &mut input,
                    &mut status,
                    &mut theme_name,
                    &mut info_screen,
                );
                match eff {
                    Effect::SwitchModel(name) => {
                        match submit(neo_protocol::Op::ConfigureSession {
                            patch: neo_protocol::SessionPatch {
                                model: Some(name.clone()),
                                ..Default::default()
                            },
                        }) {
                            Ok(produced) => {
                                events.extend(produced);
                                about.current_model = name.clone();
                                status = format!("已切换到 {name}");
                            }
                            Err(e) => status = format!("切换失败：{e}"),
                        }
                    }
                    Effect::SwitchSession(id) => {
                        if let Some(msg) = handle_session_effect(
                            &Effect::SwitchSession(id),
                            sessions,
                            &mut events,
                            &mut view_state,
                        ) {
                            status = msg;
                        }
                    }
                    Effect::SetBackground(b) => {
                        current_appearance.background = b;
                        appearance::save_preference(current_appearance);
                    }
                    Effect::SetLogo(l) => {
                        current_appearance.logo = l;
                        appearance::save_preference(current_appearance);
                    }
                    // 服务商：编辑打开表单；删除走确认。两者都必须显式处理 ——
                    // 落到 `_` catch-all 会变成静默无操作（真实踩过：删除按了没反应，
                    // 因为 Effect::RemoveProvider 没有分支、被 `_` 吞掉，且编译器不报错）。
                    Effect::EditProvider(name) => {
                        settings_form = Some(provider_form(Some(&name), providers));
                    }
                    Effect::RemoveProvider(name) => {
                        confirm_delete = Some(format!("删除服务商 {name}？（连带其密钥）"));
                        // 确认时要真正删除的名字：暂存在一个独立槽里，
                        // 避免让展示文案参与逻辑（文案改了就不能删了）。
                        pending_delete = Some(name);
                    }
                    // 主题在 apply_popup_item 里已直接改好 theme_name 并落盘
                    _ => {}
                }
                // 值变了：刷新设置页显示（保留光标位置）
                if settings_state.is_some() {
                    open_settings(
                        &mut settings_state,
                        &about,
                        &events,
                        &mut display,
                        sidebar_open,
                        mouse_on,
                        clipboard.as_ref(),
                        notify_backend.as_ref(),
                        notify_enabled,
                        notify_sound,
                        theme_name,
                        current_appearance,
                        custom_bg.is_some(),
                        provider_count,
                        sessions,
                    );
                }
            }
            // 服务商表单打开时**独占输入**（它是模态：底下列表不可点）。
            // 与内联选择器同一页，不跳出去 —— 与"设置页全屏"的范式一致。
            if let Some(form) = settings_form.as_mut() {
                match read_key_timeout(&mut stdin, 1) {
                    None => { dirty = true; continue; }
                    Some(Key::Quit) => break,
                    Some(Key::Escape) => {
                        settings_form = None;
                        status = "已取消".to_string();
                    }
                    Some(Key::Tab) | Some(Key::Down) => {
                        form.active = (form.active + 1) % form.fields.len().max(1);
                    }
                    Some(Key::Up) => {
                        let n = form.fields.len().max(1);
                        form.active = (form.active + n - 1) % n;
                    }
                    // Enter 保存：**校验失败要留住表单**并说明原因，
                    // 关掉表单再报错会让用户重填一遍（真实体验过的那种糟）。
                    Some(Key::Enter) => {
                        let name = form.trimmed(0);
                        if name.is_empty() {
                            status = "服务商名字不能为空".to_string();
                        } else {
                            form_commit = Some(form.clone());
                            settings_form = None;
                        }
                    }
                    Some(Key::Backspace) => {
                        if let Some(f) = form.fields.get_mut(form.active) { f.editor.backspace(); }
                    }
                    Some(Key::Delete) => {
                        if let Some(f) = form.fields.get_mut(form.active) { f.editor.delete_forward(); }
                    }
                    Some(Key::Left) => {
                        if let Some(f) = form.fields.get_mut(form.active) { f.editor.move_left(); }
                    }
                    Some(Key::Right) => {
                        if let Some(f) = form.fields.get_mut(form.active) { f.editor.move_right(); }
                    }
                    Some(Key::Home) => {
                        if let Some(f) = form.fields.get_mut(form.active) { f.editor.move_home(); }
                    }
                    Some(Key::End) => {
                        if let Some(f) = form.fields.get_mut(form.active) { f.editor.move_end(); }
                    }
                    // 其余可打印字符进当前字段。**不做字符白名单**：
                    // 密钥/URL 里出现什么字符都不奇怪，拦反而会挡住合法输入。
                    Some(Key::Char(c)) => {
                        if let Some(f) = form.fields.get_mut(form.active) { f.editor.insert_char(c); }
                    }
                    Some(_) => {}
                }
                dirty = true;
                continue;
            }
            // 表单保存：在读取输入**之前**落实（与 picker_apply 同一个教训 ——
            // 状态在上一轮置好，下一轮读键前就该执行，否则会被 None 超时吞掉）。
            // 删除确认：y 确认 / 其它取消
            if let Some(question) = confirm_delete.clone() {
                let _ = &question;
                match read_key_timeout(&mut stdin, 1) {
                    None => { dirty = true; continue; }
                    Some(Key::Quit) => break,
                    Some(Key::Char('y')) | Some(Key::Char('Y')) => {
                        let name = pending_delete.take().unwrap_or_default();
                        status = match providers.delete(&name) {
                            Ok(true) => format!("已删除服务商 {name}（已从模型列表移除）"),
                            Ok(false) => format!("服务商 {name} 已不存在"),
                            Err(e) => format!("删除失败：{e}"),
                        };
                        confirm_delete = None;
                        // 数量变了，刷新设置页
                        open_settings(
                            &mut settings_state, &about, &events, &mut display,
                            sidebar_open, mouse_on, clipboard.as_ref(),
                            notify_backend.as_ref(), notify_enabled, notify_sound,
                            theme_name, current_appearance, custom_bg.is_some(),
                            providers.list().len(), sessions,
                        );
                    }
                    Some(_) => {
                        status = "已取消删除".to_string();
                        confirm_delete = None;
                        pending_delete = None;
                    }
                }
                dirty = true;
                continue;
            }
            // 表单保存：在读取输入**之前**落实（与 picker_apply 同一个教训 ——
            // 状态在上一轮置好，下一轮读键前就该执行，否则会被 None 超时吞掉）。
            if let Some(form) = form_commit.take() {
                if let Some(name) = pending_delete.take() {
                    // 不保存，先问
                    confirm_delete = Some(name);
                } else {
                status = commit_provider_form(&form, providers);
                // 新增/编辑可能引入了新模型：立刻刷新宿主手里那份列表，
                // 否则「模型」里看不到它，用户以为没生效（真实困惑过）。
                about.models = providers.models();
                about.current_model = providers.current_model();
                // 刷新设置页：服务商数量变了
                open_settings(
                    &mut settings_state,
                    &about,
                    &events,
                    &mut display,
                    sidebar_open,
                    mouse_on,
                    clipboard.as_ref(),
                    notify_backend.as_ref(),
                    notify_enabled,
                    notify_sound,
                    theme_name,
                    current_appearance,
                    custom_bg.is_some(),
                    providers.list().len(),
                    sessions,
                );
                }
            }
            // 内联选择器打开时，按键**先给选择器**：↑↓ 在候选间移动、enter 确认、
            // esc 收起。这样"选值"与"选设置行"是同一页里的两个层次，
            // 不再退回聊天画面弹框（那是用户说的"割裂"）。
            if let Some(pk) = settings_picker.as_mut() {
                match read_key_timeout(&mut stdin, 1) {
                    None => { dirty = true; continue; }
                    Some(Key::Quit) => break,
                    Some(Key::Escape) | Some(Key::Char('q')) => {
                        settings_picker = None;
                        status = "已收起候选".to_string();
                    }
                    Some(Key::Up) | Some(Key::Char('k')) => {
                        pk.selected = pk.selected.saturating_sub(1);
                    }
                    Some(Key::Down) | Some(Key::Char('j')) => {
                        if pk.selected + 1 < pk.items.len() {
                            pk.selected += 1;
                        }
                    }
                    Some(Key::Home) | Some(Key::Char('g')) => pk.selected = 0,
                    Some(Key::End) | Some(Key::Char('G')) => {
                        pk.selected = pk.items.len().saturating_sub(1)
                    }
                    Some(Key::Enter) => {
                        if let Some(item) = pk.items.get(pk.selected).cloned() {
                            settings_picker = None;
                            picker_apply = Some(item);
                        }
                    }
                    Some(_) => {}
                }
                dirty = true;
                continue;
            }
            // 键盘与鼠标合成同一个"要执行第几项"，之后的执行逻辑只有一份 ——
            // 两条输入路径各写一套分支，迟早会漂移出"点得到但回车不行"这类差异。
            let mut activate: Option<usize> = None;
            match read_key_timeout(&mut stdin, 1) {
                None => continue, // 超时：只在尺寸变化时由顶层判断
                Some(k) => match k {
                    Key::Quit => break,
                    Key::Escape | Key::Char('q') => {
                        settings_state = None;
                        status = "已关闭设置".to_string();
                    }
                    // 鼠标点击可操作行 = 光标移过去 + 回车
                    Key::Mouse(ev) => {
                        if let Some(MouseAction::SettingsRow(idx)) =
                            hit_test(&sregions, &ev, None, false)
                        {
                            settings_cursor = idx;
                            activate = Some(idx);
                        }
                    }
                    // 光标只在**可操作行**之间移动 —— 停在只读行上会让
                    // 用户以为按 enter 能改点什么。
                    Key::Up | Key::Char('k') => {
                        settings_cursor = settings_cursor.saturating_sub(1);
                    }
                    Key::Down | Key::Char('j') => {
                        if settings_cursor + 1 < slots.len() {
                            settings_cursor += 1;
                        }
                    }
                    Key::Home | Key::Char('g') => settings_cursor = 0,
                    Key::End | Key::Char('G') => {
                        settings_cursor = slots.len().saturating_sub(1)
                    }
                    Key::Enter => activate = Some(settings_cursor),
                    _ => {}
                },
            }
            if let Some(activated) = activate {
                if let Some((si, ri)) = slots.get(activated).copied() {
                            if let Some(action) =
                                sections.get(si).and_then(|s| s.rows.get(ri)).and_then(|r| r.action)
                            {
                                match action {
                                    SettingAction::NextTheme => {
                                        theme_name = theme_name.next();
                                        theme::save_preference(theme_name);
                                        status = format!("主题：{}", theme_name.as_str());
                                    }
                                    SettingAction::ThemePicker => {
                                        settings_picker = Some(SettingsPicker {
                                            label: "主题".into(),
                                            items: popup::theme_items(""),
                                            selected: 0,
                                        });
                                    }
                                    SettingAction::ToggleDetails => {
                                        display.expanded = !display.expanded;
                                    }
                                    SettingAction::ToggleThinking => {
                                        display.thinking = !display.thinking;
                                    }
                                    SettingAction::ToggleSidebar => {
                                        sidebar_open = !sidebar_open;
                                    }
                                    SettingAction::Rewind => {
                                        do_rewind(&mut submit, &mut events, &mut status);
                                    }
                                    SettingAction::NewSession => {
                                        events.clear();
                                        status = "新对话（已清空转录；文件改动不受影响）"
                                            .to_string();
                                    }
                                    // 与 `/new` 走**同一条**路径（handle_session_effect），
                                    // 不在这里复制一份 —— 两处各写一遍必然漂移。
                                    SettingAction::NewSessionReal => {
                                        if let Some(msg) = handle_session_effect(
                                            &Effect::NewSessionReal,
                                            sessions,
                                            &mut events,
                                            &mut view_state,
                                        ) {
                                            status = msg;
                                        }
                                        settings_state = None;
                                    }
                                    SettingAction::DiffViewer => {
                                        open_diff_viewer(&events, &mut diff_viewer);
                                        settings_state = None;
                                    }
                                    SettingAction::ToggleNotify => {
                                        notify_enabled = !notify_enabled;
                                        // 刚打开时立刻发一条，让用户确认"能响"
                                        if notify_enabled {
                                            let _ = notify_backend.notify(
                                                neo_platform::Attention::ApprovalNeeded,
                                                "提醒已开启（试音）",
                                            );
                                        }
                                    }
                                    SettingAction::ToggleNotifySound => {
                                        notify_sound = !notify_sound;
                                    }
                                    SettingAction::NextBackground => {
                                        current_appearance.background =
                                            current_appearance.background.next();
                                        appearance::save_preference(current_appearance);
                                    }
                                    SettingAction::NextLogo => {
                                        current_appearance.logo =
                                            current_appearance.logo.next();
                                        appearance::save_preference(current_appearance);
                                    }
                                    SettingAction::SessionPicker => {
                                        settings_picker = Some(SettingsPicker {
                                            label: "会话 ID".into(),
                                            items: popup::session_items(
                                                &sessions.list(),
                                                &sessions.current(),
                                            ),
                                            selected: 0,
                                        });
                                    }
                                    SettingAction::ProviderPicker => {
                                        // 候选由契据提供；选中后进入编辑表单。
                                        // **空列表不开空弹窗**：一个"0 项"的候选框
                                        // 只会让人以为坏了 —— 直接说怎么开始。
                                        let list = providers.list();
                                        if list.is_empty() {
                                            status =
                                                "还没有服务商，用「新增服务商」添加".to_string();
                                        } else {
                                            settings_picker = Some(SettingsPicker {
                                                label: "服务商".into(),
                                                items: list
                                                    .into_iter()
                                                    .map(|(n, d, has_key)| {
                                                        let keymark = if has_key {
                                                            "已配密钥"
                                                        } else {
                                                            "缺密钥"
                                                        };
                                                        popup::Item::plain(
                                                            n.clone(),
                                                            format!("{d} · {keymark}"),
                                                            popup::ItemAction::RunProvider(n),
                                                        )
                                                    })
                                                    .collect(),
                                                selected: 0,
                                            });
                                        }
                                    }
                                    SettingAction::ProviderAdd => {
                                        settings_form = Some(provider_form(None, providers));
                                    }
                                    SettingAction::ProviderEdit => {
                                        // 没有服务商时如实说，不开一个空表单
                                        let list = providers.list();
                                        match list.first() {
                                            Some((n, _, _)) => {
                                                settings_form =
                                                    Some(provider_form(Some(n), providers));
                                            }
                                            None => {
                                                status = "还没有服务商，先用「新增服务商」".to_string()
                                            }
                                        }
                                    }
                                    SettingAction::ProviderDelete => {
                                        let list = providers.list();
                                        if list.is_empty() {
                                            status = "还没有服务商可删".to_string();
                                        } else {
                                            settings_picker = Some(SettingsPicker {
                                                label: "删除服务商".into(),
                                                items: list
                                                    .into_iter()
                                                    .map(|(n, d, _)| {
                                                        popup::Item::plain(
                                                            n.clone(),
                                                            d,
                                                            popup::ItemAction::DeleteProvider(n),
                                                        )
                                                    })
                                                    .collect(),
                                                selected: 0,
                                            });
                                        }
                                    }
                                    SettingAction::ModelPicker => {
                                        settings_picker = Some(SettingsPicker {
                                            label: "模型".into(),
                                            items: popup::model_items(
                                                &about.models,
                                                &about.current_model,
                                            ),
                                            selected: 0,
                                        });
                                    }
                                }
                                // 刷新设置页上的值（改完之后数字/状态要跟着变）。
                                // **不复位光标** —— 复位会让画面像"闪一下没反应"。
                                // 内联选择器不关设置页，所以这里始终会刷新到。
                                if settings_state.is_some() {
                                    open_settings(
                                        &mut settings_state,
                                        &about,
                                        &events,
                                        &mut display,
                                        sidebar_open,
                                        mouse_on,
                                        clipboard.as_ref(),
                                        notify_backend.as_ref(),
                                        notify_enabled,
                                        notify_sound,
theme_name,
current_appearance,
custom_bg.is_some(),
                                        provider_count,
                                        sessions,
                                    );
                                }
                }
            }
            }
            // 设置分支自己渲染并 `continue`，走不到下面那句统一的 `dirty = true`。
            // 不在这里置位的话：关掉设置（或切换模型/会话）后主界面**不会重绘**，
            // 屏幕上留着的还是设置页那一帧 —— 表现为"退出了却还看到旧画面，
            // 过一会儿才更新"（脱节）。任何在设置页里发生的输入都可能改了状态，
            // 所以统统一置位，代价只是一次重绘。
            dirty = true;
            continue;
        }

        // ── 全屏 diff 查看器：独占输入 ──────────────────────────────
        if let Some(v) = diff_viewer.as_mut() {
            let screen = Screen {
                cols,
                rows,
                facts: &[],
                input: &empty_input,
                status: "",
                awaiting_input: false,
                approval: None,
                show_cursor: false,
                about: Some(&about),
                trust: None,
                theme: theme_name,
                popup: None,
                preformatted: None,
                sidebar: false,
                view: None,
                diff_viewer: Some(v),
                whichkey: None,
                display: ToolDisplay::default(),
                settings: None,
                settings_cursor: 0,
            settings_form: None, settings_confirm: None,
                settings_picker: None,
                appearance: current_appearance,
                custom_background: custom_bg.as_ref(),
            };
            write!(stdout, "{}", screen.render())?;
            stdout.flush()?;
            let viewport = rows.saturating_sub(3);
            match read_key_timeout(&mut stdin, 1) {
                None => continue,
                Some(k) => {
                    match k {
                        Key::Quit => break,
                        Key::Escape | Key::Char('q') => {
                            diff_viewer = None;
                            status = "已关闭 diff 查看器".to_string();
                        }
                        Key::Char('j') | Key::Down => v.scroll(1, viewport),
                        Key::Char('k') | Key::Up => v.scroll(-1, viewport),
                        Key::PageDown | Key::Char(' ') => {
                            v.scroll((viewport / 2).max(1) as isize, viewport)
                        }
                        Key::PageUp => v.scroll(-((viewport / 2).max(1) as isize), viewport),
                        Key::Char('g') | Key::Home => v.set_cursor(0, viewport),
                        Key::Char('G') | Key::End => {
                            let last = v.line_count().saturating_sub(1);
                            v.set_cursor(last, viewport);
                        }
                        Key::Char(']') => v.hunk_step(true, viewport),
                        Key::Char('[') => v.hunk_step(false, viewport),
                        Key::Char('n') => v.file_step(true, viewport),
                        Key::Char('p') => v.file_step(false, viewport),
                        Key::Char('N') => {
                            // 文件树里上下选择
                            v.tree_step(1, viewport)
                        }
                        Key::Char('P') => v.tree_step(-1, viewport),
                        Key::Char('v') => {
                            v.toggle_mode();
                            let m = match v.mode {
                                diffview::ViewMode::Unified => "统一",
                                diffview::ViewMode::Split => "双列",
                            };
                            status = format!("diff 视图：{m}");
                        }
                        Key::Char('b') => v.toggle_tree(),
                        Key::Char('d') => {
                            diff_viewer = None;
                            status = "已关闭 diff 查看器".to_string();
                        }
                        _ => {}
                    }
                }
            }
            continue;
        }

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
                input: &empty_input,
                status: "按任意键返回",
                awaiting_input: false,
                approval: None,
                show_cursor: false,
                about: None,
                trust: None,
                theme: theme_name,
                popup: None,
                // 借 info 屏这段：直接用 facts 通道塞不进去（Fact 无原文类型），
                // 改用 preformatted 字段承载
                preformatted: Some(&body),
                sidebar: false,
                view: None,
                diff_viewer: None,
                whichkey: None,
                display: ToolDisplay::default(),
                settings: None,
                settings_cursor: 0,
            settings_form: None, settings_confirm: None,
                settings_picker: None,
                appearance: appearance::Appearance::default(),
                custom_background: None,
            };
            write!(stdout, "{}", screen.render())?;
            stdout.flush()?;
            match read_key_timeout(&mut stdin, 1) {
                // 超时：不重绘（避免 10Hz 刷屏），只在下轮顶部检查尺寸
                None => continue,
                Some(Key::Quit) => break,
                Some(_) => {
                    info_screen = None;
                    continue;
                }
            }
        }

        if dirty {
            let facts = facts_of(&events);
            // 提醒：扫"新出现"的完成/出错/审批信号。用 set 记住已提醒过的，
            // 避免每帧重扫 facts 时重复响。
            if notify_enabled {
                let seen: &std::collections::HashSet<String> = &notified;
                for (key, kind, detail) in attention_signals(&facts, seen) {
                    // 提醒失败**只忽略、不中断** —— 见 platform::Notify 的契约
                    let _ = notify_backend.notify(kind, &detail);
                    notified.insert(key);
                }
                // 有界：只保留最近 64 个 key，避免长会话无界增长
                if notified.len() > 64 {
                    notified.clear();
                }
            }

            let screen = Screen {
                cols,
                rows,
                facts: &facts,
                input: &input,
                status: &status,
                awaiting_input: outstanding.is_some(),
                approval: approval.as_ref(),
                show_cursor: true,
                about: Some(&about),
                trust: None,
                theme: theme_name,
                popup: popup_state.as_ref(),
                preformatted: None,
                sidebar: sidebar_open,
                view: Some(&view_state),
                diff_viewer: diff_viewer.as_ref(),
                whichkey: whichkey_groups.as_deref(),
                display,
                settings: settings_state.as_ref(),
                settings_cursor,
                settings_picker: None,
            settings_form: None, settings_confirm: None,
                appearance: current_appearance,
                custom_background: custom_bg.as_ref(),
            };
            let (out, regs) = screen.render_with_regions();
            write!(stdout, "{out}")?;
            stdout.flush()?;
            last_regions = regs;
            dirty = false;
        }

        // 带超时读键：无输入时返回 None，于是每 0.1 秒醒一次检查窗口尺寸。
        // 这是"resize 后能重绘"的唯一途径 —— 终端不会为 resize 产生输入。
        let Some(key) = read_key_timeout(&mut stdin, 1) else {
            if terminal_size() != (cols, rows) {
                dirty = true; // 尺寸变了才重绘（否则保持静止）
            }
            continue;
        };
        // 有任何按键进来都要重绘（状态可能已变）
        dirty = true;
        // which-key 是"看一眼"的提示：任何**其它**键都把它关掉，
        // 否则它会一直挂在屏幕上挡住正文（用户以为界面卡了）。
        if whichkey_groups.is_some() && !matches!(key, Key::WhichKey) {
            whichkey_groups = None;
        }

        // ── 审批对话框打开时，按键**独占**交给它 ────────────────────
        //
        // 放在弹窗/输入框之前：审批是"必须先回答才能继续"的模态，
        // 让 y/n/a 成为**一次按键**（而不是"打进输入框再回车"）。
        if approval.is_some() {
            // 先算出"这次按键要做什么"，再动 approval ——
            // 持着 `as_mut()` 的借用时不能同时把 approval 传给助手函数。
            let mut decision: Option<neo_protocol::Decision> = None;
            {
                let ap = approval.as_mut().expect("上面已判 is_some");
                match key {
                    Key::Quit => break,
                    Key::Up | Key::Char('k') => {
                        ap.selected = ap.selected.saturating_sub(1);
                    }
                    Key::Down | Key::Char('j') | Key::Tab => {
                        if ap.selected + 1 < ApprovalPrompt::CHOICES.len() {
                            ap.selected += 1;
                        }
                    }
                    // y / a / n **一次按键就生效**：选项文案里写着（y）/（a）/（n），
                    // 若只移动高亮就与文案不符 —— 用户按 y 以为批了、实际还要回车，
                    // 这正是"按了 y 像没反应"的来源。
                    Key::Char('y') | Key::Char('Y') => {
                        ap.selected = 0;
                        decision = Some(ap.choice_at());
                    }
                    Key::Char('a') | Key::Char('A') => {
                        ap.selected = 1;
                        decision = Some(ap.choice_at());
                    }
                    Key::Char('n') | Key::Char('N') => {
                        ap.selected = 2;
                        decision = Some(ap.choice_at());
                    }
                    Key::Enter | Key::Char(' ') => decision = Some(ap.choice_at()),
                    // Esc：收起卡片但**不作答** —— 待审批仍在（outstanding 未清），
                    // 用户可以先看正文/滚动，回头再答。
                    Key::Escape => {
                        status = "审批仍待处理（y 批准 / a 总是 / n 拒绝）".to_string();
                    }
                    _ => {}
                }
            }
            if let Some(d) = decision {
                decide_approval(&mut approval, &mut outstanding, &mut events, &mut submit, d);
                if outstanding.is_some() {
                    approval = build_approval_prompt(&events);
                }
                status = idle_or_approval(&outstanding);
            }
            dirty = true;
            continue;
        }
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
                    input.backspace();
                    let still = self_popup_context(&input.text());
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
                    input.insert_char(c);
                    match self_popup_context(&input.text()) {
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
                        let eff = apply_popup_item(
                            &item,
                            &mut input,
                            &mut status,
                            &mut theme_name,
                            &mut info_screen,
                        );
                        match eff {
                            Effect::Quit => should_quit = true,
                            Effect::Rewind => {
                                do_rewind(&mut submit, &mut events, &mut status)
                            }
                            Effect::ToggleDetails => {
                                display.expanded = !display.expanded;
                                status = if display.expanded {
                                    "工具输出：展开".into()
                                } else {
                                    "工具输出：折叠（失败时仍会展示）".into()
                                };
                            }
                            Effect::ToggleThinking => {
                                display.thinking = !display.thinking;
                                status = if display.thinking {
                                    "推理过程：显示".into()
                                } else {
                                    "推理过程：隐藏".into()
                                };
                            }
                            Effect::CopyLastReply => {
                                status = do_copy(&events, clipboard.as_ref())
                            }
                            Effect::Settings => {
                                open_settings_fresh(
                                    &mut settings_state,
                                    &mut settings_cursor,
                                    &about,
                                    &events,
                                    &mut display,
                                    sidebar_open,
                                    mouse_on,
                                    clipboard.as_ref(),
                                    notify_backend.as_ref(),
                                    notify_enabled,
                                    notify_sound,
theme_name,
current_appearance,
custom_bg.is_some(),
                                    provider_count,
                                    sessions,
                                );
                            }
                            Effect::ToggleSidebar => {
                                sidebar_open = !sidebar_open;
                            }
                            Effect::ToggleNotify => {
                                notify_enabled = !notify_enabled;
                                status = if notify_enabled {
                                    "提醒：开启（完成 / 出错 / 需审批）".into()
                                } else {
                                    "提醒：关闭".into()
                                };
                            }
                            Effect::ToggleNotifySound => {
                                notify_sound = !notify_sound;
                                status = if notify_sound { "提醒声音：开".into() } else { "提醒声音：关".into() };
                            }
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
                            Effect::ShowDiff => open_diff_viewer(&events, &mut diff_viewer),
                            // 外观类 Effect 统一处理（避免在每处弹窗分支重复一遍）
                            other => {
                                if let Effect::CompactSubmit = &other {
                                    match submit(neo_protocol::Op::Compact) {
                                        Ok(produced) => {
                                            events.extend(produced);
                                            status = "上下文已压缩".to_string();
                                        }
                                        Err(e) => status = format!("压缩失败：{e}"),
                                    }
                                } else if let Some(m) = handle_session_effect(
                                    &other,
                                    sessions,
                                    &mut events,
                                    &mut view_state,
                                ) {
                                    status = m;
                                } else if let Effect::SwitchModel(name) = &other {
                                    match submit(neo_protocol::Op::ConfigureSession {
                                        patch: neo_protocol::SessionPatch {
                                            model: Some(name.clone()),
                                            ..Default::default()
                                        },
                                    }) {
                                        Ok(produced) => {
                                            events.extend(produced);
                                            // 同步 About：模型名是从 CLI 注入的快照，
                                            // 不更新的话设置页/弹窗会一直显示启动时那个模型
                                            // （切换成功了但界面说旧名字 = 两处各说一套）。
                                            about.current_model = name.clone();
                                            status = format!("已切换到 {name}");
                                        }
                                        Err(e) => status = format!("切换失败：{e}"),
                                    }
                                } else if let Some(m) = handle_ui_effect(
                                    &other,
                                    &mut current_appearance,
                                    &mut popup_state,
                                    &about,
                                    sessions,
                                ) {
                                    status = m;
                                }
                            },
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
            Key::Suspend => {
                // 挂起回 shell：把终端**完整还原**后把前台交还给用户
                //（`fg` 恢复）。这是终端程序的基本礼貌 —— 用户想临时敲个
                // shell 命令不该被迫退出再重进（丢失会话）。
                raw.restore();
                mouse.leave();
                alt.leave();
                stdout.flush()?;

                // 没有 libc，用 `kill` 给**自己**发 SIGTSTP。
                // 进程会停在这次调用里，直到用户 `fg` 触发 SIGCONT。
                let pid = std::process::id().to_string();
                let _ = std::process::Command::new("kill")
                    .args(["-TSTP", &pid])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();

                // ── 已恢复（SIGCONT 之后）──
                // 终端可能被 shell 改过（尺寸、模式），所以整个重进一遍。
                match RawMode::enter() {
                    Ok(r) => raw = r,
                    Err(e) => {
                        // 恢复失败不能装作没事：如实告知并退出，否则后续
                        // 按键读不到，界面看起来"死了"。
                        eprintln!("[neo] 从挂起恢复失败：{e}");
                        break;
                    }
                }
                alt = AltScreen::enter();
                mouse = MouseMode::enter(mouse_on);
                // 强制重绘（挂起期间终端内容可能已被覆盖）
                dirty = true;
            }
            Key::WhichKey => {
                // 已显示则再按一次关掉（开关语义，避免只有"任意键关闭"一种退路）
                if whichkey_groups.is_some() {
                    whichkey_groups = None;
                    status = "已关闭键位提示".to_string();
                } else {
                    let ctx = whichkey::detect(
                        popup_state.is_some(),
                        searching,
                        outstanding.is_some(),
                        false,
                        view_state.clamped_offset(
                            transcript_line_count(&events, self_body_cols(cols, sidebar_open)),
                            rows.saturating_sub(chrome_rows(input.line_count())),
                        ) > 0,
                        sidebar_open && cols >= SIDEBAR_MIN_COLS,
                    );
                    let visible = sidebar_open && cols >= SIDEBAR_MIN_COLS;
                    whichkey_groups = Some(whichkey::groups_for(ctx, visible));
                    status = "键位提示（任意键关闭；ctrl+/ 再按一次也可）".to_string();
                }
            }

            Key::Quit => break,
            Key::Escape => {
                if searching {
                    // 退出搜索：保留滚动位置（用户可能正看着某个命中）
                    searching = false;
                    input.clear();
                    status = "已退出搜索（滚动位置保留）".to_string();
                } else {
                    // 无弹窗时 Esc 清空当前输入（与多数 shell 的 Ctrl+U 语义接近，
                    // 但不抢 ctrl+u 的键位）
                    if !input.is_empty() {
                        input.clear();
                        browsing = false;
                    }
                }
            }
            Key::Mouse(ev) => {
                if let Some(act) = hit_test(&last_regions, &ev, popup_state.as_ref(), sidebar_open) {
                    match act {
                        // 设置页是**独占输入**的分支（在它自己的循环里处理点击），
                        // 主循环渲染时 `settings: None`，故 `settings_rows` 必为空、
                        // 这个动作在此不可达。保留分支是为了让 match 穷尽 ——
                        // 将来若有人把设置改成非独占，编译器会立刻提醒这里需要接线。
                        MouseAction::SettingsRow(_) => {}
                        MouseAction::ScrollTranscript(up) => {
                            let body = rows
                                .saturating_sub(chrome_rows(input.line_count()));
                            let total = transcript_line_count(&events, self_body_cols(cols, sidebar_open));
                            if up {
                                view_state.scroll_up(3, total, body);
                            } else {
                                view_state.scroll_down(3, total, body);
                            }
                        }
                        MouseAction::SelectPopup(index) => {
                            if let Some(p) = popup_state.as_mut() {
                                p.select(index);
                            }
                            // 点选即确认（单击选中并执行的语义比"两次操作"更快）
                            let chosen = popup_state
                                .as_ref()
                                .and_then(|p| p.selected_item().cloned());
                            popup_state = None;
                            if let Some(item) = chosen {
                                let eff = apply_popup_item(
                                    &item,
                                    &mut input,
                                    &mut status,
                                    &mut theme_name,
                                    &mut info_screen,
                                );
                                match eff {
                                    Effect::Quit => break,
                                    Effect::Rewind => {
                                        do_rewind(&mut submit, &mut events, &mut status)
                                    }
                                    Effect::ToggleDetails => {
                                        display.expanded = !display.expanded;
                                        status = if display.expanded {
                                            "工具输出：展开".into()
                                        } else {
                                            "工具输出：折叠".into()
                                        };
                                    }
                                    Effect::ToggleThinking => {
                                        display.thinking = !display.thinking;
                                        status = if display.thinking {
                                            "推理过程：显示".into()
                                        } else {
                                            "推理过程：隐藏".into()
                                        };
                                    }
                                    Effect::CopyLastReply => {
                                        status = do_copy(&events, clipboard.as_ref())
                                    }
                                    Effect::Settings => {
                                        open_settings_fresh(
                                            &mut settings_state,
                                            &mut settings_cursor,
                                            &about,
                                            &events,
                                            &mut display,
                                            sidebar_open,
                                            mouse_on,
                                            clipboard.as_ref(),
                                            notify_backend.as_ref(),
                                            notify_enabled,
                                            notify_sound,
theme_name,
current_appearance,
custom_bg.is_some(),
providers.list().len(),
sessions,
                                        );
                                    }
                                    Effect::ToggleSidebar => {
                                        sidebar_open = !sidebar_open;
                                    }
                                    Effect::ToggleNotify => {
                                        notify_enabled = !notify_enabled;
                                        status = if notify_enabled {
                                            "提醒：开启（完成 / 出错 / 需审批）".into()
                                        } else {
                                            "提醒：关闭".into()
                                        };
                                    }
                                    Effect::ToggleNotifySound => {
                                        notify_sound = !notify_sound;
                                        status = if notify_sound { "提醒声音：开".into() } else { "提醒声音：关".into() };
                                    }
                                    Effect::ClearTranscript => events.clear(),
                                    Effect::ShowStatus => {
                                        info_screen = Some(commands::status_text(
                                            &about,
                                            theme_name.as_str(),
                                        ));
                                    }
                                    Effect::OpenThemePicker => {
                                        let mut tp =
                                            popup::Popup::new(popup::Kind::Theme, "");
                                        tp.set_items(popup::theme_items(""), false);
                                        popup_state = Some(tp);
                                    }
                                    Effect::ShowDiff => {
                                        open_diff_viewer(&events, &mut diff_viewer)
                                    }
                                    // 外观类 Effect 统一处理（避免在每处弹窗分支重复一遍）
                                other => {
                                    if let Effect::CompactSubmit = &other {
                                        match submit(neo_protocol::Op::Compact) {
                                            Ok(produced) => {
                                                events.extend(produced);
                                                status = "上下文已压缩".to_string();
                                            }
                                            Err(e) => status = format!("压缩失败：{e}"),
                                        }
                                    } else if let Some(m) = handle_session_effect(
                                        &other,
                                        sessions,
                                        &mut events,
                                        &mut view_state,
                                    ) {
                                        status = m;
                                    } else if let Effect::SwitchModel(name) = &other {
                                        // 模型切换要提交 Op 给内核（宿主不持有内核）
                                        match submit(neo_protocol::Op::ConfigureSession {
                                            patch: neo_protocol::SessionPatch {
                                                model: Some(name.clone()),
                                                ..Default::default()
                                            },
                                        }) {
                                            Ok(produced) => {
                                                events.extend(produced);
                                                about.current_model = name.clone();
                                                status = format!("已切换到 {name}");
                                            }
                                            Err(e) => status = format!("切换失败：{e}"),
                                        }
                                    } else if let Some(m) = handle_ui_effect(
                                        &other,
                                        &mut current_appearance,
                                        &mut popup_state,
                                        &about,
                                        sessions,
                                    ) {
                                        status = m;
                                    }
                                }
                                }
                            }
                        }
                        MouseAction::FocusInput => {
                            // 点击输入框：把注意力交回输入（清掉弹窗）
                            popup_state = None;
                        }
                        MouseAction::ToggleSidebar => {
                            sidebar_open = !sidebar_open;
                            status = if sidebar_open {
                                "侧栏已展开"
                            } else {
                                "侧栏已收起"
                            }
                            .to_string();
                        }
                    }
                }
            }
            Key::PageUp => {
                let body = rows.saturating_sub(chrome_rows(input.line_count()));
                let total = transcript_line_count(&events, cols);
                let half = body / 2;
                view_state.scroll_up(half.max(1), total, body);
            }
            Key::PageDown => {
                let body = rows.saturating_sub(chrome_rows(input.line_count()));
                let total = transcript_line_count(&events, cols);
                let half = body / 2;
                view_state.scroll_down(half.max(1), total, body);
            }
            Key::ScrollToTop => {
                let body = rows.saturating_sub(chrome_rows(input.line_count()));
                let total = transcript_line_count(&events, cols);
                view_state.to_top(total, body);
            }
            Key::ScrollToBottom => view_state.to_bottom(),
            Key::Search => {
                searching = true;
                input.clear();
                status = "搜索：输入关键词，回车确认，esc 取消".to_string();
            }
            Key::SearchNext => {
                let body = rows.saturating_sub(chrome_rows(input.line_count()));
                let total = transcript_line_count(&events, cols);
                view_state.search_step(true, total, body);
            }
            Key::SearchPrev => {
                let body = rows.saturating_sub(chrome_rows(input.line_count()));
                let total = transcript_line_count(&events, cols);
                view_state.search_step(false, total, body);
            }
            Key::DeleteToLineEnd if searching => {
                // 搜索模式下 ctrl+k 不适用，忽略以免误改
            }
            Key::DeleteWordBackward if searching => input.delete_word_backward(),
            Key::Undo => input.undo(),
            Key::Redo => input.redo(),
            Key::Delete => {
                input.delete_forward();
            }
            Key::Home => input.move_home(),
            Key::End => input.move_end(),
            Key::WordBackward => input.word_backward(),
            Key::WordForward => input.word_forward(),
            Key::Left => {
                browsing = false;
                input.move_left();
            }
            Key::Right => {
                browsing = false;
                input.move_right();
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
                input.backspace();
                browsing = false;
                if searching {
                    let ls = searchable_lines(&facts_of(&events), cols);
                    let q = input.text();
                    view_state.set_search(&ls, &q);
                    let n = view_state.search().map(|s| s.hits.len()).unwrap_or(0);
                    status = format!("搜索「{q}」：{n} 处匹配");
                }
            }
            Key::Char('d') if outstanding.is_some() && input.is_empty() => {
                // 待审批时看**完整**改动（正文里只给前 200 行摘要）
                open_diff_viewer(&events, &mut diff_viewer);
            }

            Key::Char(c) if searching => {
                input.insert_char(c);
                // 实时把查询应用到转录（所见即所得）
                let ls = searchable_lines(&facts_of(&events), cols);
                let q = input.text();
                view_state.set_search(&ls, &q);
                let n = view_state.search().map(|s| s.hits.len()).unwrap_or(0);
                status = if q.trim().is_empty() {
                    "搜索：输入关键词，回车确认，esc 取消".to_string()
                } else if n == 0 {
                    format!("搜索「{q}」：无匹配")
                } else {
                    format!("搜索「{q}」：{n} 处匹配（回车确认 · ctrl+n/ctrl+p 切换）")
                };
            }
            Key::Char(c) => {
                input.insert_char(c);
                browsing = false;
                history.reset_cursor();
                // 输入 `@` 或 `/` 即弹出候选（对齐 opencode：输入即列表）
                // 搜索模式下不触发 —— 那时输入框装的是查询词，不是任务
                if !searching && (c == '@' || c == '/') {
                    let t = input.text();
                    open_popup_for(&t, &mut popup_state, &mut file_cache, &mut status);
                }
            }
            Key::Up => {
                // 多行输入时先在本缓冲内上移；已在首行才走历史
                let (row, _) = input.cursor();
                if row > 0 {
                    input.move_up();
                } else if input.is_empty() || browsing || row == 0 {
                    if let Some(h) = history.prev() {
                        input.set(h);
                        browsing = true;
                    }
                }
            }
            Key::Down => {
                // 同理：先在本缓冲内下移
                let (row, _) = input.cursor();
                if row + 1 < input.line_count() {
                    input.move_down();
                } else if browsing {
                    match history.next_entry() {
                        Some(h) => input.set(h),
                        None => {
                            input.clear();
                            browsing = false;
                        }
                    }
                }
            }
            Key::SearchHistory => {
                // Ctrl+R：用当前输入当查询，回填最近一条匹配（再按继续往回找）
                let needle = input.text();
                if let Some(found) = history.search(&needle) {
                    let found = found.to_string();
                    // 连续 Ctrl+R 时把游标往上挪一格，实现"继续找更早的"
                    if found == needle && !needle.is_empty() {
                        let _ = history.prev();
                    }
                    input.set(&found);
                    status = format!("历史搜索：{needle}");
                } else {
                    status = format!("历史中未找到：{needle}");
                }
                browsing = true;
            }
            Key::ExternalEditor => {
                let initial = input.text();
                if let Some(edited) = edit_externally(&initial, &raw) {
                    // 外部编辑器返回的可能是多行文本，整段取代输入（保留换行）
                    input.set(edited.trim_end());
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
                            Effect::Rewind => {
                                do_rewind(&mut submit, &mut events, &mut status)
                            }
                            Effect::ToggleDetails => {
                                display.expanded = !display.expanded;
                                status = if display.expanded {
                                    "工具输出：展开".into()
                                } else {
                                    "工具输出：折叠".into()
                                };
                            }
                            Effect::ToggleThinking => {
                                display.thinking = !display.thinking;
                                status = if display.thinking {
                                    "推理过程：显示".into()
                                } else {
                                    "推理过程：隐藏".into()
                                };
                            }
                            Effect::CopyLastReply => {
                                status = do_copy(&events, clipboard.as_ref())
                            }
                            Effect::Settings => {
                                open_settings_fresh(
                                    &mut settings_state,
                                    &mut settings_cursor,
                                    &about,
                                    &events,
                                    &mut display,
                                    sidebar_open,
                                    mouse_on,
                                    clipboard.as_ref(),
                                    notify_backend.as_ref(),
                                    notify_enabled,
                                    notify_sound,
theme_name,
current_appearance,
custom_bg.is_some(),
                                    provider_count,
                                    sessions,
                                );
                            }
                            Effect::ToggleSidebar => {
                                sidebar_open = !sidebar_open;
                            }
                            Effect::ToggleNotify => {
                                notify_enabled = !notify_enabled;
                                status = if notify_enabled {
                                    "提醒：开启（完成 / 出错 / 需审批）".into()
                                } else {
                                    "提醒：关闭".into()
                                };
                            }
                            Effect::ToggleNotifySound => {
                                notify_sound = !notify_sound;
                                status = if notify_sound { "提醒声音：开".into() } else { "提醒声音：关".into() };
                            }
                            Effect::ClearTranscript => events.clear(),
                            Effect::ShowStatus => {
                                info_screen = Some(commands::status_text(
                                    &about,
                                    theme_name.as_str(),
                                ));
                            }
                            Effect::ShowDiff => open_diff_viewer(&events, &mut diff_viewer),
                            // Tab 接受主题选择后不开新弹窗，直接生效即可
                            Effect::OpenThemePicker | Effect::None => {}
                            // 会话/压缩/外观等"其余"Effect 统一处理。
                            // 曾在这里只处理外观，导致 `/compact` 落到空分支
                            // **静默无操作** —— 用户明确要求压缩却什么都没发生。
                            other => {
                                if let Effect::CompactSubmit = &other {
                                    match submit(neo_protocol::Op::Compact) {
                                        Ok(produced) => {
                                            events.extend(produced);
                                            status = "上下文已压缩".to_string();
                                        }
                                        Err(e) => status = format!("压缩失败：{e}"),
                                    }
                                } else if let Some(m) = handle_session_effect(
                                    &other,
                                    sessions,
                                    &mut events,
                                    &mut view_state,
                                ) {
                                    status = m;
                                } else if let Some(m) = handle_ui_effect(
                                    &other,
                                    &mut current_appearance,
                                    &mut popup_state,
                                    &about,
                                    sessions,
                                ) {
                                    status = m;
                                }
                            }
                        }
                        continue;
                    }
                }
                // Tab：补全 `@` 引用（只在 @ 上下文中生效）
                let cur = input.text();
                if let Some(q) = at_query(&cur) {
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
                            let done = complete_at_token(&cur, best);
                            input.set(&done);
                            status = format!("补全：{best}{}", status);
                        }
                        None => status = format!("无匹配文件：{q}"),
                    }
                }
            }
            Key::Enter if searching => {
                // 搜索模式：Enter 只确认/跳到下一个命中，不提交任务
                searching = false;
                let n = view_state.search().map(|s| s.hits.len()).unwrap_or(0);
                let q = view_state.search().map(|s| s.query.clone()).unwrap_or_default();
                input.clear();
                if n == 0 {
                    status = format!("搜索「{q}」无匹配");
                } else {
                    let body = rows.saturating_sub(chrome_rows(1));
                    let total = transcript_line_count(&events, cols);
                    view_state.search_step(false, total, body);
                    status = format!("搜索「{q}」：{n} 处（ctrl+n 下一个 / ctrl+p 上一个）");
                }
            }
            Key::Enter => {
                // 取文本并清空输入（提交后输入框应回到空）
                let line = input.text();
                input.clear();

                // ── 有待审批：本行是审批应答 ──────────────────────────────
                if let Some(id) = outstanding.clone() {
                    let Some(ans) = parse_approval_answer(&line) else {
                        status = "请回答 y 批准 / a 总是允许这类 / n 拒绝".to_string();
                        continue;
                    };
                    outstanding = None;
                    let op = neo_protocol::Op::Approve {
                        id,
                        decision: match ans {
                            ApprovalAnswer::Allow => neo_protocol::Decision::Allow,
                            ApprovalAnswer::AllowAlways => neo_protocol::Decision::AllowAlways,
                            ApprovalAnswer::Deny => neo_protocol::Decision::Deny,
                        },
                    };
                    match submit(op) {
                        Ok(produced) => {
                            outstanding = latest_approval_id(&produced);
                            events.extend(produced);
                            // 还有下一个审批就重新开框（换内容）
                            approval = if outstanding.is_some() {
                                build_approval_prompt(&events)
                            } else {
                                None
                            };
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
                                &popup::item_of(cmd),
                                &mut input,
                                &mut status,
                                &mut theme_name,
                                &mut info_screen,
                            );
                            match eff {
                                // 直接 break 出主循环；不需要再走一遍 should_quit
                                Effect::Quit => break,
                                Effect::Rewind => {
                                    do_rewind(&mut submit, &mut events, &mut status)
                                }
                                Effect::ToggleDetails => {
                                    display.expanded = !display.expanded;
                                    status = if display.expanded {
                                        "工具输出：展开".into()
                                    } else {
                                        "工具输出：折叠".into()
                                    };
                                }
                                Effect::ToggleThinking => {
                                    display.thinking = !display.thinking;
                                    status = if display.thinking {
                                        "推理过程：显示".into()
                                    } else {
                                        "推理过程：隐藏".into()
                                    };
                                }
                                Effect::CopyLastReply => {
                                    status = do_copy(&events, clipboard.as_ref())
                                }
                                Effect::Settings => {
                                    open_settings_fresh(
                                        &mut settings_state,
                                        &mut settings_cursor,
                                        &about,
                                        &events,
                                        &mut display,
                                        sidebar_open,
                                        mouse_on,
                                        clipboard.as_ref(),
                                        notify_backend.as_ref(),
                                        notify_enabled,
                                        notify_sound,
theme_name,
current_appearance,
custom_bg.is_some(),
providers.list().len(),
sessions,
                                    );
                                }
                                Effect::ToggleSidebar => {
                                    sidebar_open = !sidebar_open;
                                }
                                Effect::ToggleNotify => {
                                    notify_enabled = !notify_enabled;
                                    status = if notify_enabled {
                                        "提醒：开启（完成 / 出错 / 需审批）".into()
                                    } else {
                                        "提醒：关闭".into()
                                    };
                                }
                                Effect::ToggleNotifySound => {
                                    notify_sound = !notify_sound;
                                    status = if notify_sound { "提醒声音：开".into() } else { "提醒声音：关".into() };
                                }
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
                                Effect::ShowDiff => open_diff_viewer(&events, &mut diff_viewer),
                                other => {
                                    if let Effect::CompactSubmit = &other {
                                        match submit(neo_protocol::Op::Compact) {
                                            Ok(produced) => {
                                                events.extend(produced);
                                                status = "上下文已压缩".to_string();
                                            }
                                            Err(e) => status = format!("压缩失败：{e}"),
                                        }
                                    } else if let Some(m) = handle_session_effect(
                                        &other,
                                        sessions,
                                        &mut events,
                                        &mut view_state,
                                    ) {
                                        status = m;
                                    } else if let Effect::SwitchModel(name) = &other {
                                        // 模型切换要提交 Op 给内核（宿主不持有内核）
                                        match submit(neo_protocol::Op::ConfigureSession {
                                            patch: neo_protocol::SessionPatch {
                                                model: Some(name.clone()),
                                                ..Default::default()
                                            },
                                        }) {
                                            Ok(produced) => {
                                                events.extend(produced);
                                                about.current_model = name.clone();
                                                status = format!("已切换到 {name}");
                                            }
                                            Err(e) => status = format!("切换失败：{e}"),
                                        }
                                    } else if let Some(m) = handle_ui_effect(
                                        &other,
                                        &mut current_appearance,
                                        &mut popup_state,
                                        &about,
                                        sessions,
                                    ) {
                                        status = m;
                                    }
                                }
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
                            input: &empty_input,
                            status: &running,
                            awaiting_input: false,
                            approval: None,
                            show_cursor: false,
                            about: Some(&about),
                            trust: None,
                            theme: theme_name,
                            popup: None,
                            preformatted: None,
                            sidebar: sidebar_open,
                            view: Some(&view_state),
                            diff_viewer: None,
                            whichkey: None,
                            display,
                            settings: None,
                            settings_cursor: 0,
            settings_form: None, settings_confirm: None,
                            settings_picker: None,
                            appearance: current_appearance,
                            custom_background: custom_bg.as_ref(),
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
                        input: &empty_input,
                        status: &status,
                        awaiting_input: false,
                        approval: None,
                        show_cursor: false,
                        // 这一帧是"运行中"过渡态：facts 通常已非空，不展示首屏
                        about: Some(&about),
                        trust: None,
                        theme: theme_name,
                        popup: None,
                        preformatted: None,
                        sidebar: sidebar_open,
                        view: Some(&view_state),
                        diff_viewer: None,
                        whichkey: None,
                        display,
                        settings: None,
                        settings_cursor: 0,
            settings_form: None, settings_confirm: None,
                        settings_picker: None,
                        appearance: current_appearance,
                        custom_background: custom_bg.as_ref(),
                    }
                    .render()
                )?;
                stdout.flush()?;

                // 引用必须在**提交前**解析：`@file` / `$skill` 的解析结果
                // 是协议语义（RefKind），四个宿主共用同一份 `parse_refs`，
                // 不会各自漂移出"某个宿主不认 `$`"这类分叉。
                match submit(neo_protocol::Op::UserTurn {
                    refs: neo_protocol::parse_refs(&line),
                    text: line,
                }) {
                    Ok(produced) => {
                        outstanding = latest_approval_id(&produced);
                        events.extend(produced);
                        // 有审批请求就**弹模态框**（而不是只把输入框变黄）——
                        // 用户明确反馈"不是那种对话框形式，和 opencode/mimo 差很远"。
                        approval = if outstanding.is_some() {
                            build_approval_prompt(&events)
                        } else {
                            None
                        };
                    }
                    Err(e) => events.push(EventMsg::Error { message: e }),
                }
                status = idle_or_approval(&outstanding);
            }
            _ => {}
        }
    }

    raw.restore();
    mouse.leave();
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
        let ed = editor::Editor::from_text(input);
        Screen {
            cols,
            rows,
            facts,
            input: &ed,
            status,
            awaiting_input: false,
            approval: None,
            show_cursor: false,
            about: None,
            trust: None,
            theme: theme::ThemeName::OpenCode,
            popup: None,
            preformatted: None,
            sidebar: false,
            view: None,
            diff_viewer: None,
            whichkey: None,
            display: ToolDisplay::default(),
            settings: None,
            settings_cursor: 0,
            settings_form: None, settings_confirm: None,
            settings_picker: None,
            appearance: appearance::Appearance::default(),
            custom_background: None,
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
            models: vec![("mock".into(), "确定性桩".into(), false)],
            current_model: "mock".into(),
            session: "neo-tui".into(),
        }
    }

    fn welcome(cols: usize, rows: usize) -> String {
        let a = about();
        Screen {
            cols,
            rows,
            facts: &[],
            input: &editor::Editor::new(),
            status: "就绪",
            awaiting_input: false,
            approval: None,
            show_cursor: false,
            about: Some(&a),
            trust: None,
            theme: theme::ThemeName::OpenCode,
            popup: None,
            preformatted: None,
            sidebar: false,
            view: None,
            diff_viewer: None,
            whichkey: None,
            display: ToolDisplay::default(),
            settings: None,
            settings_cursor: 0,
            settings_form: None, settings_confirm: None,
            settings_picker: None,
            appearance: appearance::Appearance::default(),
            custom_background: None,
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
        // 控制字节分配（曾互相冲突，见 decode_key 的说明）
        assert_eq!(decode_key(&[0x0f]), Key::WhichKey, "ctrl+o = which-key");
        assert_eq!(decode_key(&[0x1f]), Key::Undo, "ctrl+_ = 撤销");
        assert_eq!(decode_key(&[0x1a]), Key::Suspend, "ctrl+z = 挂起回 shell");
        assert_eq!(decode_key(&[0x19]), Key::Redo, "ctrl+y = 重做");
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
            input: &editor::Editor::from_text("/"),
            status: "",
            awaiting_input: false,
            approval: None,
            show_cursor: false,
            about: Some(&a),
            trust: None,
            theme: theme::ThemeName::OpenCode,
            popup: Some(&p),
            preformatted: None,
            sidebar: false,
            view: None,
            diff_viewer: None,
            whichkey: None,
            display: ToolDisplay::default(),
            settings: None,
            settings_cursor: 0,
            settings_form: None, settings_confirm: None,
            settings_picker: None,
            appearance: appearance::Appearance::default(),
            custom_background: None,
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
            category: "",
            keybinding: "",
        };
        let mut input = editor::Editor::from_text("/help");
        let mut status = String::new();
        let mut theme_name = theme::ThemeName::OpenCode;
        let mut info: Option<String> = None;
        let eff = apply_popup_item(&item, &mut input, &mut status, &mut theme_name, &mut info);
        assert!(input.is_empty(), "执行命令后输入应清空，实际 {:?}", input.text());
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
            category: "",
            keybinding: "",
        };
        let mut input = editor::Editor::from_text("@src/ma");
        let mut status = String::new();
        let mut theme_name = theme::ThemeName::OpenCode;
        let mut info: Option<String> = None;
        apply_popup_item(&item, &mut input, &mut status, &mut theme_name, &mut info);
        let t = input.text();
        assert!(t.contains("@src/main.rs"), "引用应留在输入里，实际 {t:?}");
        assert!(!t.contains("ma@"), "不应重复叠加过滤词：{t:?}");
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
            input: &editor::Editor::new(),
            status: "按任意键返回",
            awaiting_input: false,
            approval: None,
            show_cursor: false,
            about: None,
            trust: None,
            theme: theme::ThemeName::OpenCode,
            popup: None,
            preformatted: Some(&lines),
            sidebar: false,
            view: None,
            diff_viewer: None,
            whichkey: None,
            display: ToolDisplay::default(),
            settings: None,
            settings_cursor: 0,
            settings_form: None, settings_confirm: None,
            settings_picker: None,
            appearance: appearance::Appearance::default(),
            custom_background: None,
        }
        .render();
        let text = plain(&out).join("\n");
        assert!(text.contains("编程 Agent 内核"), "应显示帮助正文：{text}");
        assert!(text.contains("按任意键返回"), "应提示如何返回：{text}");
    }

    #[test]
    fn render_writes_exactly_one_screenful_of_lines() {
        // 每帧只能写**一屏**的行数（rows 行）。若写完转录又打印整块网格，
        // 就会写出约两倍的 rows 行 —— 多余的换行会把终端**逐帧上滚**，
        // 表现为内容往上漂、退出后画面错位。
        let a = about();
        for rows in [20usize, 30, 44] {
            let out = Screen {
                cols: 120, rows, facts: &[], input: &editor::Editor::new(), status: "就绪",
                awaiting_input: false, show_cursor: false,
                approval: None,
                about: Some(&a), trust: None,
                theme: theme::ThemeName::OpenCode, popup: None, preformatted: None,
                sidebar: true,
                view: None,
                diff_viewer: None,
                whichkey: None,
                display: ToolDisplay::default(),
                settings: None,
                settings_cursor: 0,
            settings_form: None, settings_confirm: None,
                settings_picker: None,
                appearance: appearance::Appearance::default(),
                custom_background: None,
            }
            .render();
            let newlines = out.matches('\n').count();
            assert!(
                newlines <= rows,
                "rows={rows} 却写了 {newlines} 个换行（>rows 会逐帧滚屏）"
            );
        }
    }

    // ── 侧栏 ──────────────────────────────────────────────────────────

    #[test]
    fn no_line_exceeds_the_terminal_width_even_with_sidebar() {
        // 行超宽会让终端**折行 → 滚屏**，在备用屏里表现为"退出后残留"。
        // 这是网格模型的根本约束，必须覆盖侧栏 + 长内容 + 窄宽度的组合。
        use neo_protocol::{FileChange, TodoEntry, TodoStatus};
        let facts = vec![
            Fact::UserSaid("看下 @src/main.rs 这个文件".into()),
            Fact::AssistantSaid(
                "# 标题\n\n很长的正文，包含中文与 emoji 🚀 以及 `inline code` 与\n\n```rust\nfn main() { println!(\"hi\"); }\n```".into(),
            ),
            Fact::TodoList(vec![
                TodoEntry { content: "一项非常非常非常非常非常非常长的任务描述".into(), status: TodoStatus::InProgress },
            ]),
            Fact::FilesChanged(vec![
                FileChange { path: "src/very/deeply/nested/module/with/long/name/file.rs".into(), additions: 123, deletions: 45 },
            ]),
            Fact::PatchPreview { path: "a.txt".into(), diff: "--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-x\n+y".into() },
            Fact::ApprovalNeeded { detail: "写入类调用需确认".into() },
            Fact::Failed("一段很长的错误信息，用于测试右侧面板与正文的边界情况".into()),
            Fact::TurnFinished { input_tokens: 123456, output_tokens: 7890 },
        ];
        for cols in [96usize, 100, 120, 150, 200, 260] {
            for rows in [20usize, 40, 60] {
                let a = About { context_limit: 64_000, ..about() };
                let out = Screen {
                    cols, rows, facts: &facts, input: &editor::Editor::from_text("输入中文测试"), status: "就绪",
                    awaiting_input: true, show_cursor: true,
                    approval: None,
                    about: Some(&a), trust: None,
                    theme: theme::ThemeName::OpenCode, popup: None, preformatted: None,
                    sidebar: true,
                    view: None,
                    diff_viewer: None,
                    whichkey: None,
                    display: ToolDisplay::default(),
                    settings: None,
                    settings_cursor: 0,
            settings_form: None, settings_confirm: None,
                    settings_picker: None,
                    appearance: appearance::Appearance::default(),
                    custom_background: None,
                }
                .render();
                for (i, l) in plain(&out).iter().enumerate() {
                    let w = width::display_width(l);
                    assert!(
                        w <= cols,
                        "{cols}x{rows} 第 {i} 行宽 {w} 超过 {cols}（会折行→滚屏）：{l:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn popup_lines_also_stay_within_width() {
        // 弹窗同理：它在窄终端里最容易超宽
        let a = about();
        for cols in [96usize, 110, 140] {
            let mut p = popup::Popup::new(popup::Kind::Slash, "");
            p.set_items(popup::slash_items(""), false);
            let out = Screen {
                cols, rows: 40, facts: &[], input: &editor::Editor::from_text("/"), status: "",
                awaiting_input: false, show_cursor: false,
                approval: None,
                about: Some(&a), trust: None,
                theme: theme::ThemeName::OpenCode, popup: Some(&p), preformatted: None,
                sidebar: true,
                view: None,
                diff_viewer: None,
                whichkey: None,
                display: ToolDisplay::default(),
                settings: None,
                settings_cursor: 0,
            settings_form: None, settings_confirm: None,
                settings_picker: None,
                appearance: appearance::Appearance::default(),
                custom_background: None,
            }
            .render();
            for (i, l) in plain(&out).iter().enumerate() {
                let w = width::display_width(l);
                assert!(w <= cols, "{cols} 第 {i} 行宽 {w} 超宽：{l:?}");
            }
        }
    }


    fn sidebar_screen(cols: usize, facts: &[Fact], sidebar: bool) -> String {
        let a = About { context_limit: 64_000, ..about() };
        Screen {
            cols,
            rows: 30,
            facts,
            input: &editor::Editor::new(),
            status: "就绪",
            awaiting_input: false,
            approval: None,
            show_cursor: false,
            about: Some(&a),
            trust: None,
            theme: theme::ThemeName::OpenCode,
            popup: None,
            preformatted: None,
            sidebar,
            view: None,
            diff_viewer: None,
            whichkey: None,
            display: ToolDisplay::default(),
            settings: None,
            settings_cursor: 0,
            settings_form: None, settings_confirm: None,
            settings_picker: None,
            appearance: appearance::Appearance::default(),
            custom_background: None,
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
            cols: 120, rows: 30, facts: &facts, input: &editor::Editor::new(), status: "",
            awaiting_input: false, show_cursor: false,
            approval: None,
            about: Some(&a), trust: None,
            theme: theme::ThemeName::OpenCode, popup: None, preformatted: None,
            sidebar: true,
            view: None,
            diff_viewer: None,
            whichkey: None,
            display: ToolDisplay::default(),
            settings: None,
            settings_cursor: 0,
            settings_form: None, settings_confirm: None,
            settings_picker: None,
            appearance: appearance::Appearance::default(),
            custom_background: None,
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
            cols: 120, rows: 30, facts: &facts, input: &editor::Editor::new(), status: "",
            awaiting_input: false, show_cursor: false,
            approval: None,
            about: Some(&a), trust: None,
            theme: theme::ThemeName::OpenCode, popup: None, preformatted: None,
            sidebar: true,
            view: None,
            diff_viewer: None,
            whichkey: None,
            display: ToolDisplay::default(),
            settings: None,
            settings_cursor: 0,
            settings_form: None, settings_confirm: None,
            settings_picker: None,
            appearance: appearance::Appearance::default(),
            custom_background: None,
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
                    cols: 80, rows: 20, facts: &[], input: &editor::Editor::new(), status: "",
                    awaiting_input: false, show_cursor: false,
                    approval: None,
                    about: Some(&a), trust: None,
                    theme: t, popup: None, preformatted: None, sidebar: false,
                    view: None,
                    diff_viewer: None,
                    whichkey: None,
                    display: ToolDisplay::default(),
                    settings: None,
                    settings_cursor: 0,
            settings_form: None, settings_confirm: None,
                    settings_picker: None,
                    appearance: appearance::Appearance::default(),
                    custom_background: None,
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

    #[test]
    fn popup_shrinks_to_fit_instead_of_disappearing() {
        // 矮终端下弹窗装不下时，必须**收缩**而不是不画 ——
        // 不画的话用户按了 `/` 什么都没出现，看起来像功能坏了。
        let a = about();
        let ed = editor::Editor::new();
        let mut p = popup::Popup::new(popup::Kind::Slash, "");
        p.set_items(popup::slash_items(""), false);
        let (out, _) = Screen {
            cols: 100, rows: 14, facts: &[], input: &ed, status: "",
            awaiting_input: false, show_cursor: false,
            approval: None,
            about: Some(&a), trust: None,
            theme: theme::ThemeName::Neo, popup: Some(&p), preformatted: None,
            sidebar: false, view: None, diff_viewer: None, whichkey: None, display: ToolDisplay::default(), settings: None, settings_cursor: 0, settings_picker: None, settings_form: None, settings_confirm: None, appearance: appearance::Appearance::default(), custom_background: None,
        }
        .render_with_regions();
        let text = plain(&out).join("\n");
        assert!(text.contains('╭'), "矮终端也必须画出弹窗：{text}");
        assert!(text.contains("命令"), "至少应显示标题：{text}");
    }

    // ── 外观（背景 / Logo 样式）──────────────────────────────────────

    fn welcome_with(ap: appearance::Appearance, cols: usize, rows: usize) -> String {
        let a = about();
        let ed = editor::Editor::new();
        plain(&Screen {
            cols, rows, facts: &[], input: &ed, status: "就绪",
            awaiting_input: false, show_cursor: false,
            approval: None,
            about: Some(&a), trust: None,
            theme: theme::ThemeName::Neo, popup: None, preformatted: None,
            sidebar: false, view: None, diff_viewer: None, whichkey: None,
            display: ToolDisplay::default(), settings: None, settings_cursor: 0, settings_picker: None, settings_form: None, settings_confirm: None,
            appearance: ap, custom_background: None,
        }.render()).join("\n")
    }

    #[test]
    fn logo_style_controls_the_wordmark_size() {
        let mut ap = appearance::Appearance::default();
        // Large（默认）：6 行大词标
        ap.logo = appearance::LogoStyle::Large;
        let large = welcome_with(ap, 100, 40);
        assert!(large.contains('╗'), "Large 应显示大词标：{large}");
        // Small：3 行小词标
        ap.logo = appearance::LogoStyle::Small;
        let small = welcome_with(ap, 100, 40);
        assert!(small.contains("█▀▀█"), "Small 应显示小词标：{small}");
        assert!(!small.contains('╗'), "Small 不该出现大词标：{small}");
        // Minimal：单行
        ap.logo = appearance::LogoStyle::Minimal;
        let min = welcome_with(ap, 100, 40);
        assert!(min.contains("NEO"), "Minimal 应显示一行 NEO：{min}");
        assert!(!min.contains("█▀▀█"), "Minimal 不该有小词标：{min}");
        // Hidden：没有 logo 字符
        ap.logo = appearance::LogoStyle::Hidden;
        let hid = welcome_with(ap, 100, 40);
        assert!(!hid.contains("█▀▀█") && !hid.contains('╗'), "Hidden 不该有 logo：{hid}");
        // 但首屏信息仍在
        assert!(hid.contains("输入任务") || hid.contains("版本"), "Hidden 仍要显示首屏信息");
    }

    #[test]
    fn background_style_changes_the_texture() {
        let mut ap = appearance::Appearance::default();
        ap.logo = appearance::LogoStyle::Hidden; // 去掉 logo 干扰，只看背景
        ap.background = appearance::Background::Stars;
        let stars = welcome_with(ap, 100, 30);
        ap.background = appearance::Background::Dots;
        let dots = welcome_with(ap, 100, 30);
        ap.background = appearance::Background::None;
        let none = welcome_with(ap, 100, 30);

        assert_ne!(stars, dots, "不同背景应渲出不同纹理");
        assert!(stars.contains('·') || stars.contains('+'), "星场应有点：{stars}");
        assert!(dots.contains('·'), "点阵应有点：{dots}");
        // 纯色：没有任何纹理字符（只剩内容）
        // 纯色：**空白区域**不该有背景字符。
        // 不能整段断言"没有 ·"—— 欢迎页副标题里本来就有 `·`（"Rust 内核 · TUI"），
        // 那是内容不是背景。这里只检查"没有内容的行"。
        for l in none.lines() {
            let trimmed = l.trim();
            if trimmed.is_empty() {
                continue;
            }
            // 有内容的行不参与判定；只看那些"只有背景字符"的行
            let only_bg = trimmed.chars().all(|c| c == '·' || c == '+' || c == '\\');
            assert!(!only_bg, "纯色下出现了纯背景行：{l:?}");
        }
    }

    #[test]
    fn background_never_covers_content() {
        // 铁律：装饰只填"从未被写入"的格子，绝不盖内容
        let mut ap = appearance::Appearance::default();
        for bg in appearance::Background::all() {
            ap.background = bg;
            let t = welcome_with(ap, 100, 30);
            assert!(t.contains("输入任务"), "{bg:?} 背景盖住了输入框：{t}");
            assert!(t.contains("Neo"), "{bg:?} 背景盖住了品牌名");
        }
    }

    #[test]
    fn custom_background_replaces_the_builtin_texture() {
        let a = about();
        let ed = editor::Editor::new();
        let art: Vec<String> = vec!["XXXX".into(), "    ".into()];
        let out = plain(&Screen {
            cols: 60, rows: 20, facts: &[], input: &ed, status: "",
            awaiting_input: false, show_cursor: false,
            approval: None,
            about: Some(&a), trust: None,
            theme: theme::ThemeName::Neo, popup: None, preformatted: None,
            sidebar: false, view: None, diff_viewer: None, whichkey: None,
            display: ToolDisplay::default(), settings: None, settings_cursor: 0, settings_picker: None, settings_form: None, settings_confirm: None,
            appearance: appearance::Appearance {
                background: appearance::Background::Stars,
                logo: appearance::LogoStyle::Hidden,
            },
            custom_background: Some(&art),
        }.render()).join("\n");
        assert!(out.contains('X'), "自定义字符画应出现在背景：{out}");
        // 自定义画里的空格表示"透空"，不该有字符
        assert!(out.contains("输入任务"), "内容仍应可见");
    }

    #[test]
    fn appearance_preference_round_trips() {
        // 落盘后再读回应一致（用独立 NEO_HOME 避免污染真实配置）
        let dir = std::env::temp_dir().join(format!("neo-appear-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = std::env::var_os("NEO_HOME");
        std::env::set_var("NEO_HOME", &dir);
        let want = appearance::Appearance {
            background: appearance::Background::Diagonal,
            logo: appearance::LogoStyle::Small,
        };
        appearance::save_preference(want);
        let got = appearance::load_preference();
        match old {
            Some(v) => std::env::set_var("NEO_HOME", v),
            None => std::env::remove_var("NEO_HOME"),
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(got, want, "外观偏好应可往返");
    }

    // ── 提醒触发 ─────────────────────────────────────────────────────

    #[test]
    fn attention_fires_for_the_three_signal_kinds() {
        use neo_platform::Attention;
        let facts = vec![
            Fact::TurnFinished { input_tokens: 10, output_tokens: 2 },
            Fact::Failed("出错了".into()),
            Fact::ApprovalNeeded { detail: "写入需确认".into() },
        ];
        let seen = std::collections::HashSet::new();
        let sigs = attention_signals(&facts, &seen);
        let kinds: Vec<Attention> = sigs.iter().map(|(_, k, _)| *k).collect();
        assert!(kinds.contains(&Attention::TurnComplete), "{kinds:?}");
        assert!(kinds.contains(&Attention::Error), "{kinds:?}");
        assert!(kinds.contains(&Attention::ApprovalNeeded), "{kinds:?}");
        assert_eq!(sigs.len(), 3);
    }

    #[test]
    fn attention_does_not_refire_for_the_same_signal() {
        // 关键：facts 每帧都会被重新扫描。若不去重，同一个"完成"会响个不停。
        let facts = vec![Fact::TurnFinished { input_tokens: 1, output_tokens: 1 }];
        let mut seen = std::collections::HashSet::new();
        let first = attention_signals(&facts, &seen);
        assert_eq!(first.len(), 1, "首次应触发");
        for (k, _, _) in first {
            seen.insert(k);
        }
        let second = attention_signals(&facts, &seen);
        assert!(second.is_empty(), "已提醒过的不该再触发：{second:?}");
    }

    #[test]
    fn attention_ignores_ordinary_conversation_facts() {
        // 普通对话/工具调用不该触发提醒 —— 那会变成噪声
        let facts = vec![
            Fact::UserSaid("你好".into()),
            Fact::AssistantSaid("你好".into()),
            Fact::ToolFinished {
                name: "bash".into(), exit_code: 0,
                stdout: String::new(), stderr: String::new(), truncated: false,
            },
            Fact::FilesChanged(vec![]),
            Fact::TodoList(vec![]),
        ];
        let seen = std::collections::HashSet::new();
        assert!(attention_signals(&facts, &seen).is_empty(), "普通事实不该提醒");
    }

    #[test]
    fn attention_detail_is_bounded() {
        // 超长错误信息不该原样塞进通知（通知中心显示不下，且可能很长）
        let long = "错误".repeat(500);
        let facts = vec![Fact::Failed(long)];
        let sigs = attention_signals(&facts, &std::collections::HashSet::new());
        let (_, _, detail) = &sigs[0];
        assert!(detail.chars().count() <= 120, "提醒文案应截断，实际 {}", detail.chars().count());
    }

    // ── 剪贴板复制 ───────────────────────────────────────────────────

    /// 假的剪贴板后端：记录被复制的内容，并可模拟失败。
    struct FakeClipboard {
        got: std::sync::Mutex<Option<String>>,
        fail: bool,
    }
    impl neo_platform::Clipboard for FakeClipboard {
        fn name(&self) -> &'static str { "fake" }
        fn available(&self) -> bool { !self.fail }
        fn copy(&self, text: &str) -> Result<(), String> {
            if self.fail {
                return Err("模拟失败".into());
            }
            *self.got.lock().unwrap() = Some(text.to_string());
            Ok(())
        }
    }

    #[test]
    fn copy_takes_the_latest_reply_only() {
        let evs = vec![
            EventMsg::AgentMessageDone { text: "第一条".into() },
            EventMsg::AgentMessageDone { text: "第二条".into() },
        ];
        let cb = FakeClipboard { got: std::sync::Mutex::new(None), fail: false };
        let msg = do_copy(&evs, &cb);
        assert_eq!(cb.got.lock().unwrap().as_deref(), Some("第二条"), "应复制最近一条");
        assert!(msg.contains("已复制"), "{msg}");
    }

    #[test]
    fn copy_skips_reasoning_and_takes_the_answer() {
        // 用户要复制的是**答复**，不是思考过程
        let evs = vec![
            EventMsg::ReasoningDelta { delta: "我在想...".into() },
            EventMsg::AgentMessageDone { text: "答案是 2".into() },
        ];
        let cb = FakeClipboard { got: std::sync::Mutex::new(None), fail: false };
        do_copy(&evs, &cb);
        assert_eq!(cb.got.lock().unwrap().as_deref(), Some("答案是 2"));
    }

    #[test]
    fn copy_with_nothing_to_copy_says_so() {
        let cb = FakeClipboard { got: std::sync::Mutex::new(None), fail: false };
        let msg = do_copy(&[], &cb);
        assert!(msg.contains("没有可复制"), "{msg}");
        assert!(cb.got.lock().unwrap().is_none(), "不该写剪贴板");
    }

    #[test]
    fn copy_failure_is_reported_not_hidden() {
        // 关键：剪贴板写失败必须如实说。显示"已复制"却粘出旧内容是欺骗。
        let evs = vec![EventMsg::AgentMessageDone { text: "内容".into() }];
        let cb = FakeClipboard { got: std::sync::Mutex::new(None), fail: true };
        let msg = do_copy(&evs, &cb);
        assert!(msg.contains("复制失败"), "失败必须明说：{msg}");
        assert!(msg.contains("模拟失败"), "应带上具体原因：{msg}");
    }

    #[test]
    fn copy_ignores_empty_replies() {
        let evs = vec![
            EventMsg::AgentMessageDone { text: "   ".into() },
            EventMsg::AgentMessageDone { text: "".into() },
        ];
        let cb = FakeClipboard { got: std::sync::Mutex::new(None), fail: false };
        let msg = do_copy(&evs, &cb);
        assert!(msg.contains("没有可复制"), "空白回复不算可复制内容：{msg}");
    }

    // ── 工具输出 / 推理的显隐 ─────────────────────────────────────────

    fn tool_fact(ok: bool, out: &str) -> Vec<Fact> {
        vec![Fact::ToolFinished {
            name: "bash".into(),
            exit_code: if ok { 0 } else { 1 },
            stdout: out.into(),
            stderr: String::new(),
            truncated: false,
        }]
    }

    fn render_with_display(facts: &[Fact], disp: ToolDisplay) -> String {
        let a = about();
        let ed = editor::Editor::new();
        Screen {
            cols: 100, rows: 30, facts, input: &ed, status: "",
            awaiting_input: false, show_cursor: false,
            approval: None,
            about: Some(&a), trust: None,
            theme: theme::ThemeName::Neo, popup: None, preformatted: None,
            sidebar: false, view: None, diff_viewer: None, whichkey: None,
            display: disp,
            settings: None,
            settings_cursor: 0,
            settings_form: None, settings_confirm: None,
            settings_picker: None,
            appearance: appearance::Appearance::default(),
            custom_background: None,
        }
        .render()
    }

    #[test]
    fn tool_output_is_hidden_by_default_but_counted() {
        // 默认折叠：输出常常很长，铺开会淹没对话。
        // 但必须给出"藏了多少行"的提示，否则用户不知道有东西没显示。
        let facts = tool_fact(true, "第一行\n第二行\n第三行");
        let text = plain(&render_with_display(&facts, ToolDisplay::default())).join("\n");
        assert!(!text.contains("第一行"), "默认不该展开输出：{text}");
        assert!(text.contains("3 行输出"), "应提示有多少行输出：{text}");
        assert!(text.contains("/details"), "应提示怎么展开：{text}");
    }

    #[test]
    fn details_expands_the_output() {
        let facts = tool_fact(true, "打印内容甲\n打印内容乙");
        let disp = ToolDisplay { expanded: true, thinking: false };
        let text = plain(&render_with_display(&facts, disp)).join("\n");
        assert!(text.contains("打印内容甲"), "展开后应看到输出：{text}");
        assert!(text.contains("打印内容乙"), "{text}");
    }

    #[test]
    fn failed_tool_output_is_shown_even_when_collapsed() {
        // 关键：失败时**强制展示** —— 出错还把原因藏起来，用户只能靠猜。
        let facts = tool_fact(false, "错误详情：找不到文件");
        let text = plain(&render_with_display(&facts, ToolDisplay::default())).join("\n");
        assert!(text.contains("错误详情"), "失败输出必须可见：{text}");
    }

    #[test]
    fn long_tool_output_is_bounded_and_reported() {
        let long: String = (0..200).map(|i| format!("行{i}\n")).collect();
        let facts = tool_fact(true, &long);
        let disp = ToolDisplay { expanded: true, thinking: false };
        let text = plain(&render_with_display(&facts, disp)).join("\n");
        assert!(text.contains("另有"), "超上限应如实标注：{text}");
        // 不能把 200 行全铺出来
        assert!(!text.contains("行199"), "不该全部展开：{text}");
    }

    #[test]
    fn truncated_output_is_labelled_as_truncated_not_as_more_lines() {
        // "被内核截断"与"还有更多行没显示"是两件事，说法必须不同 ——
        // 说成后者会让用户以为展开就能看到全部。
        let facts = vec![Fact::ToolFinished {
            name: "bash".into(),
            exit_code: 0,
            stdout: "部分输出".into(),
            stderr: String::new(),
            truncated: true,
        }];
        let disp = ToolDisplay { expanded: true, thinking: false };
        let text = plain(&render_with_display(&facts, disp)).join("\n");
        assert!(text.contains("已被截断"), "应说明输出被截断：{text}");
    }

    #[test]
    fn reasoning_is_hidden_unless_enabled() {
        let facts = vec![
            Fact::AssistantThought("我先分析一下需求".into()),
            Fact::AssistantSaid("答复正文".into()),
        ];
        let off = plain(&render_with_display(&facts, ToolDisplay::default())).join("\n");
        assert!(!off.contains("我先分析"), "默认不显示推理：{off}");
        assert!(off.contains("答复正文"), "答复始终可见：{off}");

        let on = plain(&render_with_display(
            &facts,
            ToolDisplay { expanded: false, thinking: true },
        ))
        .join("\n");
        assert!(on.contains("我先分析"), "开启后应显示推理：{on}");
        assert!(on.contains("答复正文"), "{on}");
    }

    #[test]
    fn reasoning_and_answer_are_separately_controllable() {
        // 分开建模的理由：混成一个 Fact 就无法独立控制显隐
        let facts = vec![
            Fact::AssistantThought("推理".into()),
            Fact::AssistantSaid("答复".into()),
        ];
        let disp = ToolDisplay { expanded: true, thinking: false };
        let text = plain(&render_with_display(&facts, disp)).join("\n");
        assert!(text.contains("答复"), "{text}");
        assert!(!text.contains("推理"), "展开工具输出不该连带显示推理：{text}");
    }

    // ── which-key ────────────────────────────────────────────────────

    #[test]
    fn palette_sections_fit_inside_the_card_without_leaking() {
        // 实测 bug：分组标题占了行但高度计算没算进去 → 内容溢出框外，
        // 底下欢迎页的文字透上来（看到 `/undo` 行前面挂着 `v0.1`）。
        // 这条断言覆盖"面板内容必须完全落在框内"。
        let a = about();
        let ed = editor::Editor::new();
        let mut p = popup::Popup::new(popup::Kind::Palette, "");
        p.set_items(popup::palette_items(""), false);
        for rows in [18usize, 22, 30, 44] {
            let out = Screen {
                cols: 120, rows, facts: &[], input: &ed, status: "就绪",
                awaiting_input: false, show_cursor: true,
                approval: None,
                about: Some(&a), trust: None,
                theme: theme::ThemeName::Neo, popup: Some(&p), preformatted: None,
                sidebar: false, view: None, diff_viewer: None, whichkey: None,
                display: ToolDisplay::default(), settings: None, settings_cursor: 0, settings_picker: None, settings_form: None, settings_confirm: None, appearance: appearance::Appearance::default(), custom_background: None,
            }
            .render();
            let lines = plain(&out);
            // 卡片是**居中**的，所以不能要求行首就是框线。改为量列：
            // 先由顶部边框行确定卡片的 [x0, x1] 列区间，再断言区间内的每一行
            // 都以框线字符起止（否则就是内容溢出框外、或背景文字透了进来）。
            let border_row = lines.iter().find(|l| l.contains('╭')).expect("应有上边框");
            let x0 = border_row.find('╭').unwrap();
            let x1 = border_row.rfind('╮').unwrap();
            let top = lines.iter().position(|l| l.contains('╭')).unwrap();
            let bottom = lines.iter().rposition(|l| l.contains('╰')).unwrap();
            for l in &lines[top..=bottom] {
                let chars: Vec<char> = l.chars().collect();
                if chars.len() <= x1 {
                    continue;
                }
                let first = chars[x0];
                let last = chars[x1];
                assert!(
                    matches!(first, '│' | '┃' | '╭' | '╰'),
                    "rows={rows} 卡片左边有框外内容（会漏出背景）：{l:?}"
                );
                assert!(
                    matches!(last, '│' | '┃' | '╮' | '╯'),
                    "rows={rows} 卡片右边有框外内容：{l:?}"
                );
            }
            // 「推荐」分组标题必须出现（说明分组渲染生效）
            assert!(
                lines[top..=bottom].iter().any(|l| l.contains("推荐")),
                "rows={rows} 应显示分组标题"
            );
        }
    }

    #[test]
    fn palette_shows_keybinding_hints() {
        // 面板右侧显示键位，让"命令"与"键盘"对上号
        let a = about();
        let ed = editor::Editor::new();
        let mut p = popup::Popup::new(popup::Kind::Palette, "");
        p.set_items(popup::palette_items(""), false);
        // 键位需要右侧空间；窄终端里会被让位给标签（这是刻意的取舍），
        // 所以用宽终端断言"宽敞时给出键位提示"。
        let out = Screen {
            cols: 150, rows: 46, facts: &[], input: &ed, status: "",
            awaiting_input: false, show_cursor: true,
            approval: None,
            about: Some(&a), trust: None,
            theme: theme::ThemeName::Neo, popup: Some(&p), preformatted: None,
            sidebar: false, view: None, diff_viewer: None, whichkey: None,
            display: ToolDisplay::default(), settings: None, settings_cursor: 0, settings_picker: None, settings_form: None, settings_confirm: None, appearance: appearance::Appearance::default(), custom_background: None,
        }
        .render();
        let text = plain(&out).join("\n");
        assert!(text.contains("ctrl+t"), "应显示主题的键位：{text}");
    }

    // ── 设置视图 ─────────────────────────────────────────────────────

    #[test]
    fn settings_lists_the_four_sections() {
        let info = SettingsInfo {
            notify: true, notify_sound: true, notify_enabled: false,
        background: "stars".into(), logo: "large".into(), custom_background: false,
        model_count: 3, provider_count: 2,
        session_count: 2,
            version: "0.1.0".into(), binary_built: "test".into(), model: "deepseek".into(), mode: "default".into(),
            workspace: "/tmp/ws".into(), branch: "main".into(), session: "s1".into(),
            context_limit: 64_000, theme: "neo".into(),
            details: false, thinking: false, sidebar: true, mouse: true, clipboard: true,
            messages: 4, files_changed: 2,
        };
        let secs = settings_sections(&info);
        assert_eq!(secs.len(), 4, "应有系统/模型/会话/显示四节");
        let titles: Vec<&str> = secs.iter().map(|s| s.title).collect();
        assert_eq!(titles, vec!["系统", "模型", "会话", "显示"]);
    }

    #[test]
    fn settings_only_makes_achievable_items_clickable() {
        // 关键：服务商 / 模型 / 上下文上限在运行中改不了，
        // 必须标为只读并给出原因 —— 假控件比缺控件更糟。
        let info = SettingsInfo {
            notify: true, notify_sound: true, notify_enabled: false,
        background: "stars".into(), logo: "large".into(), custom_background: false,
        model_count: 3, provider_count: 2,
        session_count: 2,
            version: "0.1.0".into(), binary_built: "test".into(), model: "deepseek".into(), mode: "default".into(),
            workspace: "/tmp/ws".into(), branch: "main".into(), session: "s1".into(),
            context_limit: 64_000, theme: "neo".into(),
            details: false, thinking: false, sidebar: true, mouse: true, clipboard: true,
            messages: 4, files_changed: 0,
        };
        let secs = settings_sections(&info);
        let find = |label: &str| {
            secs.iter()
                .flat_map(|s| s.rows.iter())
                .find(|r| r.label == label)
                .unwrap_or_else(|| panic!("应有设置项 {label}"))
                .clone()
        };
        // 模型切换现在**可操作**（本轮补的后端能力：内核持注册表）
        let m = find("模型");
        assert!(m.action.is_some(), "模型应可切换（内核已支持运行时切换）");
        // 真正改不了的仍标只读并给出原因
        for label in ["档位", "上下文上限"] {
            let r = find(label);
            assert!(r.action.is_none(), "{label} 不该可点击");
            assert!(!r.readonly_note.is_empty(), "{label} 只读项必须给出原因");
        }
        for label in ["主题", "侧栏", "工具输出", "推理过程", "模型"] {
            let r = find(label);
            assert!(r.action.is_some(), "{label} 应当可操作");
        }
    }

    #[test]
    fn settings_view_renders_and_stays_within_width() {
        let a = about();
        let ed = editor::Editor::new();
        let info = SettingsInfo {
        notify: true, notify_sound: true, notify_enabled: false,
        background: "stars".into(), logo: "large".into(), custom_background: false,
        model_count: 3, provider_count: 2,
        session_count: 2,
            version: "0.1.0".into(), binary_built: "test".into(), model: "deepseek-chat".into(), mode: "default".into(),
            workspace: "/Volumes/data1/work/office/debug/proteus-code".into(),
            branch: "main".into(), session: "neo-tui".into(),
            context_limit: 64_000, theme: "neo".into(),
            details: false, thinking: false, sidebar: true, mouse: true, clipboard: true,
            messages: 12, files_changed: 3,
        };
        let secs = settings_sections(&info);
        for cols in [70usize, 100, 140, 200] {
            let out = Screen {
                cols, rows: 40, facts: &[], input: &ed, status: "",
                awaiting_input: false, show_cursor: false,
                approval: None,
                about: Some(&a), trust: None,
                theme: theme::ThemeName::Neo, popup: None, preformatted: None,
                sidebar: false, view: None, diff_viewer: None, whichkey: None,
                display: ToolDisplay::default(), settings: Some(&secs), settings_cursor: 0,
                settings_picker: None,
            settings_form: None, settings_confirm: None,
                appearance: appearance::Appearance::default(), custom_background: None,
            }
            .render();
            let text = plain(&out).join("\n");
            assert!(text.contains("设置"), "cols={cols} 应显示设置标题：{text}");
            for (i, l) in plain(&out).iter().enumerate() {
                let w = width::display_width(l);
                assert!(w <= cols, "cols={cols} 第 {i} 行宽 {w} 超宽");
            }
        }
    }

    #[test]
    fn cursor_column_uses_display_width_for_wide_chars() {
        // 中文一个字占**2 列**，而 editor.cursor() 的列是**字符下标**。
        // 直接用字符下标当列偏移，输入「你是谁」三个字后光标列=3，
        // 实际应落在第 6 列 —— 会停在第二个字「是」的中间（用户截图的现象）。
        // 这条测试从渲染输出里解析硬件光标位置（ESC[r;cH）来验。
        let a = about();
        let render_with = |text: &str| {
            let mut ed = editor::Editor::new();
            ed.set(text);
            let out = Screen {
                cols: 120, rows: 30, facts: &[], input: &ed, status: "就绪",
                awaiting_input: false, show_cursor: true,
                approval: None,
                about: Some(&a), trust: None,
                theme: theme::ThemeName::Neo, popup: None, preformatted: None,
                sidebar: false, view: None, diff_viewer: None, whichkey: None,
                display: ToolDisplay::default(), settings: None, settings_cursor: 0,
                settings_picker: None,
            settings_form: None, settings_confirm: None,
                appearance: appearance::Appearance::default(), custom_background: None,
            }
            .render();
            // 取**最后一个**光标定位序列：它是渲染末尾的 `ESC[r;cH`。
            // 从后往前扫，并按"数字;数字H"的形态严格匹配 —— 先前扫的写法
            // 会把正文里任意一个 'H' 当成终止符、跨过多个转义序列，解析必失败。
            let mut last = None;
            for chunk in out.rsplit("\x1b[") {
                let digits: String = chunk
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == ';')
                    .collect();
                if !digits.contains(';') || !chunk[digits.len()..].starts_with('H') {
                    continue;
                }
                if let Some((r, c)) = digits.split_once(';') {
                    if let (Ok(r), Ok(c)) = (r.parse::<usize>(), c.parse::<usize>()) {
                        last = Some((r, c));
                        break;
                    }
                }
            }
            last.expect("show_cursor=true 应输出光标定位序列")
        };
        let (_, c_ascii) = render_with("abc");   // 3 个半角 → 3 列
        let (_, c_cjk) = render_with("你是谁");   // 3 个全角 → 6 列
        assert_eq!(c_cjk - c_ascii, 3, "全角应比同字数半角多占 3 列（每字 +1）");
        // 再直接钉死绝对值：文本从框内第 2 列（0基）开始 ⇒ 1 基 col = left+3+6。
        // 用两串的长度差反推 left，避免依赖居中算式。
        let (_, c_two) = render_with("你好");     // 4 列
        assert_eq!(c_cjk - c_two, 2, "每多一个全角字应 +2 列");
    }

    #[test]
    fn settings_cursor_index_space_matches_render_highlight() {
        // 渲染高亮与 Enter 必须落在**同一行**。只读行插在中间时，若一边数全部行、
        // 另一边只数可操作行，就会出现"高亮在这一项、回车却动了那一项"。
        // 逐个光标位置渲染，断言 ▸ 所在行正是 Enter 会作用的行。
        let info = SettingsInfo {
            notify: true, notify_sound: true, notify_enabled: false,
            background: "stars".into(), logo: "large".into(), custom_background: false,
            model_count: 3, provider_count: 2,
            session_count: 2,
            version: "0.1.0".into(), binary_built: "test".into(), model: "d".into(), mode: "m".into(),
            workspace: "/w".into(), branch: "".into(), session: "s".into(),
            context_limit: 0, theme: "neo".into(),
            details: false, thinking: false, sidebar: true, mouse: true, clipboard: true,
            messages: 0, files_changed: 0,
        };
        let secs = settings_sections(&info);
        let slots = settings_actionable(&secs);
        assert!(slots.len() > 3, "需要足够多的可操作项才能暴露错位");

        for cursor in 0..slots.len() {
            let out = Screen {
                cols: 140, rows: 60, facts: &[], input: &ed_empty(), status: "",
                awaiting_input: false, show_cursor: false,
                approval: None,
                about: None, trust: None,
                theme: theme::ThemeName::Neo, popup: None, preformatted: None,
                sidebar: false, view: None, diff_viewer: None, whichkey: None,
                display: ToolDisplay::default(), settings: Some(&secs), settings_cursor: cursor,
                settings_picker: None,
            settings_form: None, settings_confirm: None,
                appearance: appearance::Appearance::default(), custom_background: None,
            }
            .render();
            let lines = plain(&out);
            // 底部提示行里也有一个 ▸（说明文字），排除它只数真正的行高亮
            let marked: Vec<&String> = lines
                .iter()
                .filter(|l| l.contains('▸') && !l.contains("灰字为只读项"))
                .collect();
            assert_eq!(marked.len(), 1, "cursor={cursor} 应恰有一行高亮：{marked:?}");
            let (si, ri) = slots[cursor];
            let want = &secs[si].rows[ri].label;
            assert!(
                marked[0].contains(want.as_str()),
                "cursor={cursor}: 高亮行是 {:?}，但 Enter 会作用在 {want:?}",
                marked[0]
            );
        }
    }

    fn ed_empty() -> editor::Editor {
        editor::Editor::new()
    }

    #[test]
    fn settings_view_shows_action_feedback() {
        // 设置页的部分动作（提醒开/关、提醒声音、主题）**只改状态文案**，
        // 不改任何一行的值。若设置页不把 status 画出来，按回车后界面毫无变化
        // ——用户以为没反应，其实动作已执行。这条测试守住"执行了就必须看得见"。
        let info = SettingsInfo {
            notify: true, notify_sound: true, notify_enabled: false,
            background: "stars".into(), logo: "large".into(), custom_background: false,
            model_count: 3, provider_count: 2, session_count: 2,
            version: "0.1.0".into(), binary_built: "test".into(), model: "d".into(), mode: "m".into(),
            workspace: "/w".into(), branch: "".into(), session: "s".into(),
            context_limit: 0, theme: "neo".into(),
            details: false, thinking: false, sidebar: true, mouse: true, clipboard: true,
            messages: 0, files_changed: 0,
        };
        let secs = settings_sections(&info);
        let ed = editor::Editor::new();
        let out = Screen {
            cols: 140, rows: 40, facts: &[], input: &ed,
            status: "提醒：已开启", awaiting_input: false, show_cursor: false,
            approval: None,
            about: Some(&about()), trust: None,
            theme: theme::ThemeName::Neo, popup: None, preformatted: None,
            sidebar: false, view: None, diff_viewer: None, whichkey: None,
            display: ToolDisplay::default(), settings: Some(&secs), settings_cursor: 0,
            settings_picker: None,
            settings_form: None, settings_confirm: None,
            appearance: appearance::Appearance::default(), custom_background: None,
        }
        .render();
        assert!(
            plain(&out).iter().any(|l| l.contains("提醒：已开启")),
            "设置页必须画出动作反馈，否则'执行了但看不见'"
        );
    }

    #[test]
    fn settings_feedback_does_not_overlap_the_last_row() {
        // 反馈行占 rows-2，图例占 rows-1；内容必须停在这之前。
        // 若内容画到 rows-2，反馈文字会把最后一项盖住（或反之），
        // 两者叠在一起就是"看起来乱/像没反应"。
        let info = SettingsInfo {
            notify: true, notify_sound: true, notify_enabled: false,
            background: "stars".into(), logo: "large".into(), custom_background: false,
            model_count: 3, provider_count: 2, session_count: 2,
            version: "0.1.0".into(), binary_built: "test".into(), model: "d".into(), mode: "m".into(),
            workspace: "/w".into(), branch: "".into(), session: "s".into(),
            context_limit: 0, theme: "neo".into(),
            details: false, thinking: false, sidebar: true, mouse: true, clipboard: true,
            messages: 0, files_changed: 0,
        };
        let secs = settings_sections(&info);
        let ed = editor::Editor::new();
        // 行数刻意压小，逼出"内容撞上反馈行"的情形
        for rows in [18usize, 20, 24] {
            let out = Screen {
                cols: 120, rows, facts: &[], input: &ed,
                status: "状态反馈", awaiting_input: false, show_cursor: false,
                approval: None,
                about: Some(&about()), trust: None,
                theme: theme::ThemeName::Neo, popup: None, preformatted: None,
                sidebar: false, view: None, diff_viewer: None, whichkey: None,
                display: ToolDisplay::default(), settings: Some(&secs), settings_cursor: 0,
                settings_picker: None,
            settings_form: None, settings_confirm: None,
                appearance: appearance::Appearance::default(), custom_background: None,
            }
            .render();
            let lines = plain(&out);
            assert!(lines.len() >= rows - 1, "rows={rows} 应画出底部两行");
            let fb = lines.get(rows - 2).cloned().unwrap_or_default();
            assert!(
                fb.contains("状态反馈"),
                "rows={rows}: 反馈行应是第 {} 行，实得 {fb:?}",
                rows - 2
            );
            let legend = lines.get(rows - 1).cloned().unwrap_or_default();
            assert!(legend.contains("灰字为只读项"), "rows={rows}: 图例应固定在最后一行");
        }
    }

    #[test]
    fn settings_cursor_only_lands_on_actionable_rows() {
        // 光标必须只在可操作行间移动 —— 停在只读行上会让人以为能改
        let info = SettingsInfo {
        notify: true, notify_sound: true, notify_enabled: false,
        background: "stars".into(), logo: "large".into(), custom_background: false,
        model_count: 3, provider_count: 2,
        session_count: 2,
            version: "0.1.0".into(), binary_built: "test".into(), model: "d".into(), mode: "m".into(),
            workspace: "/w".into(), branch: "".into(), session: "s".into(),
            context_limit: 0, theme: "neo".into(),
            details: false, thinking: false, sidebar: true, mouse: true, clipboard: true,
            messages: 0, files_changed: 0,
        };
        let secs = settings_sections(&info);
        let slots = settings_actionable(&secs);
        assert!(!slots.is_empty(), "应有可操作项");
        for (si, ri) in slots.iter().copied() {
            assert!(
                secs[si].rows[ri].action.is_some(),
                "settings_actionable 只应返回可操作行"
            );
        }
        // 只读行不该出现在可操作清单里
        let total_rows: usize = secs.iter().map(|s| s.rows.len()).sum();
        assert!(slots.len() < total_rows, "确实存在只读行（否则这条断言没意义）");
    }

    #[test]
    fn whichkey_overlay_renders_a_card() {
        let a = about();
        let ed = editor::Editor::new();
        let groups = whichkey::groups_for(whichkey::Context::Input, false);
        let out = Screen {
            cols: 120, rows: 34, facts: &[], input: &ed, status: "就绪",
            awaiting_input: false, show_cursor: true,
            approval: None,
            about: Some(&a), trust: None,
            theme: theme::ThemeName::Neo, popup: None, preformatted: None,
            sidebar: false, view: None, diff_viewer: None,
            whichkey: Some(&groups),
            display: ToolDisplay::default(),
            settings: None,
            settings_cursor: 0,
            settings_form: None, settings_confirm: None,
            settings_picker: None,
            appearance: appearance::Appearance::default(),
            custom_background: None,
        }
        .render();
        let text = plain(&out).join("\n");
        assert!(text.contains('╭'), "应画出卡片边框：{text}");
        assert!(text.contains("提交 / 编辑"), "应显示分组标题：{text}");
        assert!(text.contains("ctrl+k"), "应列出键位：{text}");
    }

    #[test]
    fn whichkey_overlay_does_not_cover_the_input_box() {
        // 用户按 which-key 时通常还要继续打字 → 卡片不能压在输入框上
        let a = about();
        let ed = editor::Editor::from_text("正在输入的任务");
        let groups = whichkey::groups_for(whichkey::Context::Input, false);
        let out = Screen {
            cols: 120, rows: 34, facts: &[], input: &ed, status: "",
            awaiting_input: false, show_cursor: true,
            approval: None,
            about: Some(&a), trust: None,
            theme: theme::ThemeName::Neo, popup: None, preformatted: None,
            sidebar: false, view: None, diff_viewer: None,
            whichkey: Some(&groups),
            display: ToolDisplay::default(),
            settings: None,
            settings_cursor: 0,
            settings_form: None, settings_confirm: None,
            settings_picker: None,
            appearance: appearance::Appearance::default(),
            custom_background: None,
        }
        .render();
        let text = plain(&out).join("\n");
        assert!(text.contains("正在输入的任务"), "输入内容必须仍可见：{text}");
        assert!(text.contains("default ⏵"), "输入框状态行必须仍可见：{text}");
    }

    #[test]
    fn whichkey_card_stays_within_width_and_is_opaque() {
        let a = about();
        let ed = editor::Editor::new();
        let groups = whichkey::groups_for(whichkey::Context::Input, true);
        for cols in [70usize, 96, 120, 200] {
            let out = Screen {
                cols, rows: 34, facts: &[], input: &ed, status: "",
                awaiting_input: false, show_cursor: true,
                approval: None,
                about: Some(&a), trust: None,
                theme: theme::ThemeName::Neo, popup: None, preformatted: None,
                sidebar: cols >= 96, view: None, diff_viewer: None,
                whichkey: Some(&groups),
                display: ToolDisplay::default(),
                settings: None,
                settings_cursor: 0,
            settings_form: None, settings_confirm: None,
                settings_picker: None,
                appearance: appearance::Appearance::default(),
                custom_background: None,
            }
            .render();
            let lines = plain(&out);
            for (i, l) in lines.iter().enumerate() {
                let w = width::display_width(l);
                assert!(w <= cols, "{cols} 第 {i} 行宽 {w} 超宽");
            }
            // 卡片区域内不得漏出 logo 的半块字符（不透明）
            let card_rows: Vec<&String> =
                lines.iter().filter(|l| l.contains('╭') || l.contains('│')).collect();
            for l in card_rows {
                if l.contains("提交 / 编辑") {
                    assert!(!l.contains('█'), "卡片里漏进了背景：{l:?}");
                }
            }
        }
    }

    // ── diff 查看器 ──────────────────────────────────────────────────

    fn viewer_screen(cols: usize, rows: usize, v: &diffview::Viewer) -> String {
        let a = about();
        let ed = editor::Editor::new();
        Screen {
            cols, rows, facts: &[], input: &ed, status: "",
            awaiting_input: false, show_cursor: false,
            approval: None,
            about: Some(&a), trust: None,
            theme: theme::ThemeName::Neo, popup: None, preformatted: None,
            sidebar: false, view: None, diff_viewer: Some(v), whichkey: None, display: ToolDisplay::default(), settings: None, settings_cursor: 0, settings_picker: None, settings_form: None, settings_confirm: None, appearance: appearance::Appearance::default(), custom_background: None,
        }
        .render()
    }

    fn sample_viewer() -> diffview::Viewer {
        diffview::Viewer::new(diffview::parse(
            "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,3 +1,4 @@\n fn main() {\n-    old();\n+    new();\n+    extra();\n }",
        ))
    }

    #[test]
    fn diff_viewer_renders_lines_with_markers_and_numbers() {
        let v = sample_viewer();
        let text = plain(&viewer_screen(120, 24, &v)).join("\n");
        assert!(text.contains("src/main.rs"), "应显示文件路径：{text}");
        assert!(text.contains("-"), "应显示删除标记");
        assert!(text.contains("+"), "应显示新增标记");
        assert!(text.contains("old();"), "应显示删除内容：{text}");
        assert!(text.contains("new();"), "应显示新增内容：{text}");
        assert!(text.contains("extra();"), "应显示第二处新增：{text}");
        assert!(text.contains("@@"), "应显示 hunk 头：{text}");
    }

    #[test]
    fn diff_viewer_shows_a_file_tree_on_wide_terminals() {
        // 多文件时左侧应有文件树
        let d = diffview::parse(
            "--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-x\n+y\n--- a/b.txt\n+++ b/b.txt\n@@ -1 +1 @@\n-p\n+q",
        );
        let v = diffview::Viewer::new(d);
        let wide = plain(&viewer_screen(120, 24, &v)).join("\n");
        assert!(wide.contains("a.txt") && wide.contains("b.txt"), "宽终端应列出两个文件：{wide}");
        // 窄终端不画树（否则 diff 内容没地方放）
        let narrow_screen = viewer_screen(70, 24, &v);
        let narrow = plain(&narrow_screen).join("\n");
        // 标记与内容在相邻的网格格子里，屏幕上看到的是 `+ y`（中间有列号）
        assert!(
            narrow.contains("+ y") || narrow.contains("+ q") || narrow.contains("y") ,
            "窄终端应仍能看到内容：{narrow}"
        );
    }

    #[test]
    fn diff_viewer_split_mode_puts_adds_on_the_right() {
        let v = sample_viewer();
        let uni = plain(&viewer_screen(120, 24, &v)).join("\n");
        let mut v2 = sample_viewer();
        v2.toggle_mode();
        let split = plain(&viewer_screen(120, 24, &v2)).join("\n");
        assert_ne!(uni, split, "切换视图应改变渲染");
        // 双列时新增行应出现在右半部分
        let add_row = plain(&viewer_screen(120, 24, &v2))
            .into_iter()
            .find(|l| l.contains("new();"))
            .expect("应能找到新增行");
        let pos = add_row.find("new();").unwrap();
        assert!(pos > 40, "双列视图里新增应在右半部分（实际列 {pos}）：{add_row:?}");
    }

    #[test]
    fn diff_viewer_lines_stay_within_width() {
        // 行宽不变量在查看器里同样成立（超宽会折行滚屏）
        let long = format!("+{}", "很长的中文内容".repeat(30));
        let d = diffview::parse(&format!(
            "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n{long}"
        ));
        let v = diffview::Viewer::new(d);
        for cols in [70usize, 100, 140, 200] {
            let out = viewer_screen(cols, 24, &v);
            for (i, l) in plain(&out).iter().enumerate() {
                let w = width::display_width(l);
                assert!(w <= cols, "{cols} 第 {i} 行宽 {w} 超宽：{l:?}");
            }
        }
    }

    #[test]
    fn empty_diff_viewer_renders_a_message_not_a_blank_screen() {
        // 空改动时不能是一片空白 —— 用户会以为界面坏了
        let v = diffview::Viewer::new(diffview::parse(""));
        let text = plain(&viewer_screen(100, 20, &v)).join("\n");
        assert!(text.contains("无改动"), "应说明没有改动：{text}");
        assert!(text.contains("返回"), "应提示怎么退出：{text}");
    }

    #[test]
    fn open_diff_viewer_collects_all_patch_previews() {
        // `/diff` 应汇总本会话**所有**改动，而不是只看最后一条
        let evs = vec![
            EventMsg::PatchProposed {
                path: "a.txt".into(),
                diff: "--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-x\n+y".into(),
            },
            EventMsg::PatchProposed {
                path: "b.txt".into(),
                diff: "--- a/b.txt\n+++ b/b.txt\n@@ -1 +1 @@\n-p\n+q".into(),
            },
        ];
        let mut slot = None;
        open_diff_viewer(&evs, &mut slot);
        let v = slot.expect("应打开查看器");
        assert_eq!(v.diff.files.len(), 2, "应包含两个文件");
        let text = plain(&viewer_screen(120, 24, &v)).join("\n");
        assert!(text.contains("a.txt") && text.contains("b.txt"), "{text}");
    }

    #[test]
    fn open_diff_viewer_with_no_changes_still_opens() {
        // 没改动时也应能打开（显示"无改动"），而不是静默什么都不做 ——
        // 静默会让用户以为快捷键坏了
        let mut slot = None;
        open_diff_viewer(&[], &mut slot);
        assert!(slot.is_some(), "无改动也应打开查看器并说明情况");
    }

    // ── 鼠标命中 ─────────────────────────────────────────────────────

    fn regions_for(cols: usize, rows: usize, popup: Option<&popup::Popup>) -> Regions {
        let a = about();
        let ed = editor::Editor::new();
        Screen {
            cols, rows, facts: &[], input: &ed, status: "就绪",
            awaiting_input: false, show_cursor: false,
            approval: None,
            about: Some(&a), trust: None,
            theme: theme::ThemeName::Neo, popup, preformatted: None,
            sidebar: true, view: None, diff_viewer: None, whichkey: None, display: ToolDisplay::default(), settings: None, settings_cursor: 0, settings_picker: None, settings_form: None, settings_confirm: None, appearance: appearance::Appearance::default(), custom_background: None,
        }
        .render_with_regions()
        .1
    }

    fn ev(button: mouse::Button, x: usize, y: usize, pressed: bool) -> mouse::MouseEvent {
        mouse::MouseEvent { button, pressed, x, y, modified: false }
    }

    #[test]
    fn wheel_scrolls_the_transcript() {
        let r = regions_for(140, 30, None);
        assert_eq!(
            hit_test(&r, &ev(mouse::Button::WheelUp, 20, 10, true), None, true),
            Some(MouseAction::ScrollTranscript(true))
        );
        assert_eq!(
            hit_test(&r, &ev(mouse::Button::WheelDown, 20, 10, true), None, true),
            Some(MouseAction::ScrollTranscript(false))
        );
    }

    #[test]
    fn release_events_do_nothing() {
        // 终端把一次点击拆成按下+释放；两边都处理会让动作执行两次
        let r = regions_for(140, 30, None);
        assert_eq!(hit_test(&r, &ev(mouse::Button::Left, 20, 10, false), None, true), None);
    }

    #[test]
    fn clicking_inside_the_input_box_focuses_it() {
        let cols = 140usize;
        let r = regions_for(cols, 30, None);
        let (l, rr, top, bottom) = r.input_box.expect("应有输入框区域");
        // 取输入框正中
        let x = (l + rr) / 2;
        let y = (top + bottom) / 2;
        assert_eq!(
            hit_test(&r, &ev(mouse::Button::Left, x, y, true), None, true),
            Some(MouseAction::FocusInput)
        );
    }

    #[test]
    fn clicking_a_settings_row_maps_to_that_exact_row() {
        // 鼠标点击设置行必须命中**同一项**：命中用的序号与 settings_cursor
        // 同一坐标空间，点第 k 项就该执行第 k 项（与 Enter 等价）。
        let info = SettingsInfo {
            notify: true, notify_sound: true, notify_enabled: false,
            background: "stars".into(), logo: "large".into(), custom_background: false,
            model_count: 3, provider_count: 2, session_count: 2,
            version: "0.1.0".into(), binary_built: "test".into(), model: "d".into(), mode: "m".into(),
            workspace: "/w".into(), branch: "".into(), session: "s".into(),
            context_limit: 0, theme: "neo".into(),
            details: false, thinking: false, sidebar: true, mouse: true, clipboard: true,
            messages: 0, files_changed: 0,
        };
        let secs = settings_sections(&info);
        let ed = editor::Editor::new();
        let (_, r) = Screen {
            cols: 140, rows: 40, facts: &[], input: &ed, status: "",
            awaiting_input: false, show_cursor: false,
            approval: None,
            about: Some(&about()), trust: None,
            theme: theme::ThemeName::Neo, popup: None, preformatted: None,
            sidebar: false, view: None, diff_viewer: None, whichkey: None,
            display: ToolDisplay::default(), settings: Some(&secs), settings_cursor: 0,
            settings_picker: None,
            settings_form: None, settings_confirm: None,
            appearance: appearance::Appearance::default(), custom_background: None,
        }
        .render_with_regions();
        let rows = settings_actionable(&secs);
        assert_eq!(
            r.settings_rows.len(),
            rows.len(),
            "每个可操作行都应有鼠标命中区"
        );
        for (y, idx) in &r.settings_rows {
            assert!(
                *idx < rows.len(),
                "命中序号 {idx} 越界（可操作项 {n} 个）", n = rows.len()
            );
            // 关键：用**终端坐标**（1 基）去点，而不是网格行号。
            // 上一版这里直接传 `*y`，与实现里漏掉 `+1` 的假设一致，
            // 于是测试通过但真机点击永远错一行 —— 测试复刻了实现的错误假设，
            // 等于没测。坐标契约必须按真实来源（SGR 是 1 基）来验。
            assert_eq!(
                hit_test(&r, &ev(mouse::Button::Left, 20, *y + 1, true), None, false),
                Some(MouseAction::SettingsRow(*idx)),
                "点击第 {} 行应命中第 {idx} 项",
                *y + 1
            );
        }
        // 只读行不该有命中区（点它不能触发任何动作）。
        // 行号按 draw_settings 的真实布局算：起始 row=2，每节先空一行再标题（+2），
        // 然后每行占一行。
        let actionable_ys: Vec<usize> = r.settings_rows.iter().map(|(y, _)| *y).collect();
        let mut ro_y = 2usize;
        let mut readonly_ys: Vec<usize> = Vec::new();
        for sec in &secs {
            ro_y += 2; // 节前空行 + 节标题
            for row in &sec.rows {
                if row.action.is_none() {
                    readonly_ys.push(ro_y);
                }
                ro_y += 1;
            }
        }
        for y in &readonly_ys {
            assert!(
                !actionable_ys.contains(y),
                "只读行 {y} 不该可点击"
            );
        }
        assert!(!readonly_ys.is_empty(), "用例里应存在只读行，否则这条断言没意义");
    }

    #[test]
    fn unix_seconds_format_to_civil_date() {
        // 手写日历换算最容易错在闰年与月长，用几个已知点钉住。
        assert_eq!(format_unix_utc_minute(0), "1970-01-01 00:00");
        // 2026-09-11 17:45:00 UTC
        assert_eq!(format_unix_utc_minute(1_789_148_700), "2026-09-11 17:45");
        // 闰日：2024-02-29 12:00:00 UTC
        assert_eq!(format_unix_utc_minute(1_709_208_000), "2024-02-29 12:00");
        // 非闰年 3 月 1 日（2 月只有 28 天）
        assert_eq!(format_unix_utc_minute(1_709_251_200), "2024-03-01 00:00");
        // 世纪闰年规则：2000-02-29
        assert_eq!(format_unix_utc_minute(951_782_400), "2000-02-29 00:00");
    }

    #[test]
    fn settings_page_paints_the_background_texture() {
        // 背景纹理只填 Tone::None 的格子，因此**必须在画内容之前**调用。
        // 曾写反（先画内容再填纹理）→ 一个格子都填不上，
        // 表现为"设置页里点背景只换了名字、画面没变化"。
        let info = SettingsInfo {
            notify: true, notify_sound: true, notify_enabled: false,
            background: "stars".into(), logo: "large".into(), custom_background: false,
            model_count: 3, provider_count: 2, session_count: 2,
            version: "0.1.0".into(), binary_built: "test".into(), model: "d".into(), mode: "m".into(),
            workspace: "/w".into(), branch: "".into(), session: "s".into(),
            context_limit: 0, theme: "neo".into(),
            details: false, thinking: false, sidebar: true, mouse: true, clipboard: true,
            messages: 0, files_changed: 0,
        };
        let secs = settings_sections(&info);
        let ed = editor::Editor::new();
        let render_with = |bg: appearance::Background| {
            let mut ap = appearance::Appearance::default();
            ap.background = bg;
            Screen {
                cols: 120, rows: 36, facts: &[], input: &ed, status: "",
                awaiting_input: false, show_cursor: false,
                approval: None,
                about: Some(&about()), trust: None,
                theme: theme::ThemeName::Neo, popup: None, preformatted: None,
                sidebar: false, view: None, diff_viewer: None, whichkey: None,
                display: ToolDisplay::default(), settings: Some(&secs), settings_cursor: 0,
                settings_picker: None,
            settings_form: None, settings_confirm: None,
                appearance: ap, custom_background: None,
            }
            .render()
        };
        // 星场应真的画出字符：与"无纹理"相比，画面必须不同
        assert_ne!(
            render_with(appearance::Background::Stars),
            render_with(appearance::Background::None),
            "设置页必须真的应用背景纹理（点了背景画面要变）"
        );
    }

    #[test]
    fn settings_picker_actions_are_not_swallowed_by_refresh() {
        // 记账式断言：这三项会开弹窗（把 settings_state 置 None），
        // 之后的"刷新设置页"必须**跳过**，否则设置页被又打开、弹窗永远出不来
        // —— 表现就是"选主题/选模型回车没反应"。
        // 这里直接检查值：一旦有人把它们改回"不开弹窗"的变体，测试会提醒。
        let info = SettingsInfo {
            notify: true, notify_sound: true, notify_enabled: false,
            background: "stars".into(), logo: "large".into(), custom_background: false,
            model_count: 3, provider_count: 2, session_count: 2,
            version: "0.1.0".into(), binary_built: "test".into(), model: "d".into(), mode: "m".into(),
            workspace: "/w".into(), branch: "".into(), session: "s".into(),
            context_limit: 0, theme: "neo".into(),
            details: false, thinking: false, sidebar: true, mouse: true, clipboard: true,
            messages: 0, files_changed: 0,
        };
        let secs = settings_sections(&info);
        let find = |label: &str| {
            secs.iter()
                .flat_map(|s| s.rows.iter())
                .find(|r| r.label == label)
                .and_then(|r| r.action)
        };
        assert_eq!(find("主题"), Some(SettingAction::ThemePicker));
        assert_eq!(find("模型"), Some(SettingAction::ModelPicker));
        assert_eq!(find("会话"), Some(SettingAction::SessionPicker));
    }

    #[test]
    fn settings_new_session_creates_a_real_session() {
        // "新对话"必须是**真新建会话**，不能只是清空转录：
        // 只清屏幕的话内核仍记得全部历史，用户以为开了新对话、
        // 模型却还在旧上下文里 —— 这是最危险的那种错觉。
        let info = SettingsInfo {
            notify: true, notify_sound: true, notify_enabled: false,
            background: "stars".into(), logo: "large".into(), custom_background: false,
            model_count: 3, provider_count: 2, session_count: 2,
            version: "0.1.0".into(), binary_built: "test".into(), model: "d".into(), mode: "m".into(),
            workspace: "/w".into(), branch: "".into(), session: "s".into(),
            context_limit: 0, theme: "neo".into(),
            details: false, thinking: false, sidebar: true, mouse: true, clipboard: true,
            messages: 0, files_changed: 0,
        };
        let secs = settings_sections(&info);
        let row = secs
            .iter()
            .flat_map(|s| s.rows.iter())
            .find(|r| r.label == "新对话")
            .expect("设置页应有'新对话'项");
        assert_eq!(
            row.action,
            Some(SettingAction::NewSessionReal),
            "必须是真新建会话（而非只清空转录）"
        );
    }

    #[test]
    fn settings_session_value_follows_live_session_not_stale_about() {
        // 会话名必须取自 SessionControl 的**当前值**：About.session 是启动快照，
        // 用了它就会出现"切换了会话，设置页还显示旧会话"（脱节）。
        struct FakeSessions;
        impl SessionControl for FakeSessions {
            fn list(&self) -> Vec<(String, String, usize)> { vec![] }
            fn switch(&mut self, _id: &str) -> Result<Vec<EventMsg>, String> { Ok(vec![]) }
            fn create(&mut self) -> Result<String, String> { Ok("new".into()) }
            fn delete(&mut self, _id: &str) -> Result<bool, String> { Ok(false) }
            fn current(&self) -> String { "LIVE-SESSION".into() }
        }
        let mut sess = FakeSessions;
        let a = about(); // 它的 session 字段是启动时的旧值
        let mut slot = None;
        let mut cursor = 0;
        open_settings_fresh(
            &mut slot, &mut cursor, &a, &[], &ToolDisplay::default(), true, true,
            &neo_platform::NoopClipboard::new("test"), &neo_platform::NoopNotify::new("test"),
            false, true, theme::ThemeName::Neo, appearance::Appearance::default(),
            false, 2, &mut sess,
        );
        let value = slot
            .expect("应生成设置分节")
            .iter()
            .flat_map(|s| s.rows.iter())
            .find(|r| r.label == "会话 ID")
            .map(|r| r.value.clone())
            .unwrap();
        assert_eq!(
            value, "LIVE-SESSION",
            "会话 ID 必须取自 SessionControl 的实时值，而不是 About 的启动快照"
        );
    }

    #[test]
    fn bare_escape_resolves_without_blocking() {
        // 真实 bug：read_key_timeout 读到首字节后 `stty raw` 把 min/time 复位成阻塞，
        // 于是下面为转义序列续读的第一步会**永久阻塞** —— 单独按 ESC 表现为
        // "按了没反应"（界面卡在 read 上），而方向键因字节已在缓冲里不受影响，
        // 所以这个坑藏了很久。用户的反馈是"q 能退出、esc 不行"。
        //
        // 这里用一个"续读立刻 EOF"的 reader 钉住契约：ESC 之后没有字节时，
        // decode_first 必须返回 Escape 而不是等待。
        let mut empty = std::io::empty();
        assert_eq!(
            decode_first(&[0x1b], &mut empty),
            Key::Escape,
            "单独 ESC 必须立刻解析为 Escape（不得阻塞等待续字节）"
        );
        // 有续字节时仍应正常解析为方向键（不能因为修 ESC 而破坏它们）
        let mut buf = std::io::Cursor::new(vec![b'[', b'B']);
        assert_eq!(decode_first(&[0x1b], &mut buf), Key::Down);
    }

    #[test]
    fn clicking_the_sidebar_toggles_it() {
        let r = regions_for(140, 30, None);
        let (x0, x1, _, _) = r.sidebar.expect("宽终端应有侧栏");
        let x = (x0 + x1) / 2;
        assert_eq!(
            hit_test(&r, &ev(mouse::Button::Left, x, 5, true), None, true),
            Some(MouseAction::ToggleSidebar)
        );
    }

    /// 假的服务商契据（宿主可脱离文件系统单测 —— 这正是用契据的收益）。
    struct FakeProviders {
        entries: Vec<(String, String, bool)>,
        deleted: Vec<String>,
    }
    impl ProviderControl for FakeProviders {
        fn list(&self) -> Vec<(String, String, bool)> { self.entries.clone() }
        fn models(&self) -> Vec<(String, String, bool)> { self.entries.clone() }
        fn current_model(&self) -> String { "fake".to_string() }
        fn get(&self, name: &str) -> Option<(String, String, String, u64)> {
            self.entries
                .iter()
                .find(|(n, _, _)| n == name)
                .map(|_| ("https://gw/v1".into(), "m".into(), "d".into(), 64000))
        }
        fn upsert(
            &mut self, name: &str, _b: &str, _m: &str, _k: &str, _d: &str, _l: u64,
            _orig: Option<&str>,
        ) -> Result<(), String> {
            self.entries.push((name.to_string(), String::new(), false));
            Ok(())
        }
        fn delete(&mut self, name: &str) -> Result<bool, String> {
            self.deleted.push(name.to_string());
            self.entries.retain(|(n, _, _)| n != name);
            Ok(true)
        }
    }

    #[test]
    fn provider_form_masks_the_key_field() {
        // 密钥**不能明文铺在屏幕上**：终端可能被共享/录屏。
        // 打码显示，且点数与真实长度无关（不泄露密钥长度）。
        let mut providers = FakeProviders { entries: vec![], deleted: vec![] };
        let mut form = provider_form(None, &providers);
        form.fields[3].editor.set("sk-super-secret-value");
        let pk = SettingsForm { key_field: Some(3), ..form.clone() };
        // 表单画在**设置页之内**（settings 必须为 Some，否则整个设置分支不渲染 ——
        // 第一版这里传了 None，表单根本没画出来，测试失败暴露的正是这一点）。
        let info = SettingsInfo {
            notify: true, notify_sound: true, notify_enabled: false,
            background: "stars".into(), logo: "large".into(), custom_background: false,
            model_count: 3, provider_count: 1, session_count: 2,
            version: "0.1.0".into(), binary_built: "test".into(), model: "d".into(),
            mode: "m".into(), workspace: "/w".into(), branch: "".into(),
            session: "s".into(), context_limit: 0, theme: "neo".into(),
            details: false, thinking: false, sidebar: true, mouse: true, clipboard: true,
            messages: 0, files_changed: 0,
        };
        let secs = settings_sections(&info);
        let ed = editor::Editor::new();
        let out = Screen {
            cols: 100, rows: 30, facts: &[], input: &ed, status: "",
            awaiting_input: false, show_cursor: false,
            approval: None,
            about: Some(&about()), trust: None,
            theme: theme::ThemeName::Neo, popup: None, preformatted: None,
            sidebar: false, view: None, diff_viewer: None, whichkey: None,
            display: ToolDisplay::default(), settings: Some(&secs), settings_cursor: 0,
            settings_picker: None, settings_form: Some(&pk), settings_confirm: None,
            appearance: appearance::Appearance::default(), custom_background: None,
        }
        .render();
        let text = plain(&out).join("\n");
        assert!(!text.contains("sk-super-secret-value"), "密钥不得明文出现在屏幕上");
        assert!(text.contains("••••••••"), "应显示打码占位");
        let _ = &mut providers; // 契据仍需可变（表单提交时会用到）
    }

    #[test]
    fn provider_control_contract_is_exercised_by_the_fake() {
        // 负向保护：契据的五个方法都要能被实现（否则宿主换个后端就编不过）。
        let mut p = FakeProviders { entries: vec![("a".into(), "x".into(), true)], deleted: vec![] };
        assert_eq!(p.list().len(), 1);
        assert!(p.get("a").is_some());
        p.upsert("b", "u", "m", "k", "d", 1, None).unwrap();
        assert_eq!(p.list().len(), 2);
        assert_eq!(p.delete("a").unwrap(), true);
        assert_eq!(p.deleted, vec!["a"]);
    }

    #[test]
    fn commit_form_reports_bad_context_limit_instead_of_silently_zero() {
        // 上下文上限填了非数字必须**如实报错**：静默当 0 会让用户以为填对了。
        let mut providers = FakeProviders { entries: vec![], deleted: vec![] };
        let mut form = provider_form(None, &providers);
        form.fields[0].editor.set("gw");
        form.fields[5].editor.set("abc");
        let msg = commit_provider_form(&form, &mut providers);
        assert!(msg.contains("不是数字"), "应报错而不是静默：{msg}");
        // 留空 = 未知（0），这是允许的
        form.fields[5].editor.set("");
        let msg = commit_provider_form(&form, &mut providers);
        assert!(msg.contains("已新增"), "留空应可保存：{msg}");
    }

    #[test]
    fn settings_picker_expands_inline_under_its_row() {
        // 内联选择器必须长在**那一行下面**，且不跳回聊天界面。
        // 早先是"关掉全屏设置页 → 退回聊天画面弹个小框"，观感割裂
        // （用户的反馈原话）；这条测试钉住"候选出现在设置页内部"。
        let info = SettingsInfo {
            notify: true, notify_sound: true, notify_enabled: false,
            background: "stars".into(), logo: "large".into(), custom_background: false,
            model_count: 3, provider_count: 2, session_count: 2,
            version: "0.1.0".into(), binary_built: "test".into(), model: "d".into(), mode: "m".into(),
            workspace: "/w".into(), branch: "".into(), session: "s".into(),
            context_limit: 0, theme: "neo".into(),
            details: false, thinking: false, sidebar: true, mouse: true, clipboard: true,
            messages: 0, files_changed: 0,
        };
        let secs = settings_sections(&info);
        let pk = SettingsPicker {
            label: "主题".into(),
            items: popup::theme_items(""),
            selected: 1,
        };
        let ed = editor::Editor::new();
        let out = Screen {
            cols: 120, rows: 40, facts: &[], input: &ed, status: "",
            awaiting_input: false, show_cursor: false,
            approval: None,
            about: Some(&about()), trust: None,
            theme: theme::ThemeName::Neo, popup: None, preformatted: None,
            sidebar: false, view: None, diff_viewer: None, whichkey: None,
            display: ToolDisplay::default(), settings: Some(&secs), settings_cursor: 0,
            settings_picker: Some(&pk), settings_form: None, settings_confirm: None,
            appearance: appearance::Appearance::default(), custom_background: None,
        }
        .render();
        let lines = plain(&out);
        let theme_row = lines
            .iter()
            .position(|l| l.contains("主题"))
            .expect("应有主题行");
        // 候选应出现在主题行的**下方**，且带选择箭头
        let below: Vec<&String> = lines.iter().skip(theme_row + 1).take(6).collect();
        assert!(
            below.iter().any(|l| l.contains('❯')),
            "候选应内联展开在主题行下方（含 ❯ 选择标记），实得 {below:?}"
        );
        // 且仍显示设置标题（即没有跳出设置页）
        assert!(
            lines.iter().any(|l| l.trim_start().starts_with("设置")),
            "展开候选时仍应在设置页内"
        );
    }

    #[test]
    fn a_hidden_sidebar_leaves_a_clickable_grip() {
        // 没有把手的话，鼠标用户点收起后**再也点不开**（陷阱）
        let a = about();
        let ed = editor::Editor::new();
        let (_, r) = Screen {
            cols: 140, rows: 30, facts: &[], input: &ed, status: "",
            awaiting_input: false, show_cursor: false,
            approval: None,
            about: Some(&a), trust: None,
            theme: theme::ThemeName::Neo, popup: None, preformatted: None,
            sidebar: false, view: None, diff_viewer: None, whichkey: None, display: ToolDisplay::default(), settings: None, settings_cursor: 0, settings_picker: None, settings_form: None, settings_confirm: None, appearance: appearance::Appearance::default(), custom_background: None,
        }
        .render_with_regions();
        let (gx, gy) = r.sidebar_grip.expect("侧栏收起时应留一个把手");
        assert!(r.sidebar.is_none(), "收起时不该有侧栏区域");
        assert_eq!(
            hit_test(&r, &ev(mouse::Button::Left, gx, gy, true), None, false),
            Some(MouseAction::ToggleSidebar),
            "点把手应能展开侧栏"
        );
    }

    #[test]
    fn clicking_a_popup_row_selects_that_item() {
        let mut p = popup::Popup::new(popup::Kind::Slash, "");
        p.set_items(popup::slash_items(""), false);
        let r = regions_for(140, 30, Some(&p));
        assert!(!r.popup_items.is_empty(), "应记录弹窗候选行");
        // 点第二项
        let ((y, x0, x1), index) = r.popup_items[1];
        let got = hit_test(
            &r,
            &ev(mouse::Button::Left, (x0 + x1) / 2, y + 1, true),
            Some(&p),
            true,
        );
        assert_eq!(got, Some(MouseAction::SelectPopup(index)));
        assert_eq!(index, 1, "第二个候选的下标应为 1");
    }

    #[test]
    fn popup_takes_priority_over_the_input_box() {
        // 弹窗盖在正文与输入框上方，点击必须优先给弹窗
        let mut p = popup::Popup::new(popup::Kind::Slash, "");
        p.set_items(popup::slash_items(""), false);
        let r = regions_for(140, 30, Some(&p));
        let ((y, x0, x1), _) = r.popup_items[0];
        let got = hit_test(
            &r,
            &ev(mouse::Button::Left, (x0 + x1) / 2, y + 1, true),
            Some(&p),
            true,
        );
        assert!(matches!(got, Some(MouseAction::SelectPopup(_))), "应选中弹窗项：{got:?}");
    }

    #[test]
    fn narrow_terminals_have_no_sidebar_region() {
        // 窄终端隐藏侧栏 → 不该有点击侧栏的区域（否则点在正文会切换侧栏）
        let r = regions_for(80, 30, None);
        assert!(r.sidebar.is_none(), "窄终端不该有侧栏区域");
        // 点在右侧应是空白/转录，而不是 ToggleSidebar
        let got = hit_test(&r, &ev(mouse::Button::Left, 75, 15, true), None, true);
        assert_ne!(got, Some(MouseAction::ToggleSidebar));
    }

    // ── 转录滚动与搜索 ───────────────────────────────────────────────

    #[test]
    fn scrolled_view_shows_earlier_content() {
        // 长转录：贴底时看不到开头，上滚后应能看到
        let facts: Vec<Fact> =
            (0..60).map(|i| Fact::AssistantSaid(format!("内容{i}"))).collect();
        let a = about();
        let mut v = view::View::new();
        let mk = |v: &view::View| {
            Screen {
                cols: 100, rows: 20, facts: &facts, input: &editor::Editor::new(),
                status: "", awaiting_input: false, show_cursor: false,
                approval: None,
                about: Some(&a), trust: None,
                theme: theme::ThemeName::Neo, popup: None, preformatted: None,
                sidebar: false, view: Some(v), diff_viewer: None, whichkey: None, display: ToolDisplay::default(), settings: None, settings_cursor: 0, settings_picker: None, settings_form: None, settings_confirm: None, appearance: appearance::Appearance::default(), custom_background: None,
            }
            .render()
        };
        let bottom = plain(&mk(&v)).join("\n");
        assert!(bottom.contains("内容59"), "贴底应显示最新：{bottom}");
        // 上滚
        v.scroll_up(200, 200, 15);
        let top = plain(&mk(&v)).join("\n");
        assert!(top.contains("内容0"), "上滚后应能看到开头：{top}");
        assert!(!top.contains("内容59"), "上滚后不该还显示末尾");
    }

    #[test]
    fn scrolled_view_reports_remaining_lines_below() {
        // 不在底部时必须提示"下方还有内容"，否则用户以为到底了
        let facts: Vec<Fact> =
            (0..40).map(|i| Fact::AssistantSaid(format!("行{i}"))).collect();
        let a = about();
        let mut v = view::View::new();
        v.scroll_up(10, 80, 15);
        let out = Screen {
            cols: 100, rows: 20, facts: &facts, input: &editor::Editor::new(),
            status: "", awaiting_input: false, show_cursor: false,
            approval: None,
            about: Some(&a), trust: None,
            theme: theme::ThemeName::Neo, popup: None, preformatted: None,
            sidebar: false, view: Some(&v), diff_viewer: None, whichkey: None, display: ToolDisplay::default(), settings: None, settings_cursor: 0, settings_picker: None, settings_form: None, settings_confirm: None, appearance: appearance::Appearance::default(), custom_background: None,
        }
        .render();
        assert!(plain(&out).join("\n").contains("下方还有"), "应提示下方还有内容");
    }

    #[test]
    fn search_marks_hits_and_moves_the_view() {
        let facts: Vec<Fact> =
            (0..50).map(|i| Fact::AssistantSaid(format!("第{i}条"))).collect();
        let a = about();
        let mut v = view::View::new();
        let ls = searchable_lines(&facts, 100);
        v.set_search(&ls, "第7条");
        let n = v.search().map(|s| s.hits.len()).unwrap_or(0);
        assert_eq!(n, 1, "「第7条」应恰好命中一条");
        v.search_step(false, ls.len(), 15);
        let out = Screen {
            cols: 100, rows: 20, facts: &facts, input: &editor::Editor::new(),
            status: "", awaiting_input: false, show_cursor: false,
            approval: None,
            about: Some(&a), trust: None,
            theme: theme::ThemeName::Neo, popup: None, preformatted: None,
            sidebar: false, view: Some(&v), diff_viewer: None, whichkey: None, display: ToolDisplay::default(), settings: None, settings_cursor: 0, settings_picker: None, settings_form: None, settings_confirm: None, appearance: appearance::Appearance::default(), custom_background: None,
        }
        .render();
        let t = plain(&out).join("\n");
        assert!(t.contains("第7条"), "命中行应进入视野：{t}");
        assert!(t.contains('▶'), "当前命中应有标记：{t}");
    }

    #[test]
    fn searchable_lines_reflect_what_is_on_screen() {
        // 搜索必须匹配**屏幕上可见的文字**，而不是 Debug 输出里的枚举名
        let facts = vec![Fact::AssistantSaid("代码里有个函数 foo_bar".into())];
        let ls = searchable_lines(&facts, 80).join("\n");
        assert!(ls.contains("foo_bar"), "应能搜到正文内容");
        assert!(!ls.contains("AssistantSaid"), "不该把枚举名纳入搜索：{ls}");
    }

    #[test]
    fn transcript_line_count_is_nonzero_and_bounded() {
        let evs = vec![EventMsg::AgentMessageDone { text: "a\nb\nc".into() }];
        let n = transcript_line_count(&evs, 80);
        assert!(n >= 3, "三行文本至少算 3 行，实际 {n}");
        assert!(n < 1000, "估算不该失控");
    }

    // ── 多行输入（行高动态）────────────────────────────────────────

    #[test]
    fn input_box_grows_with_multiline_content() {
        // 贴多行内容时输入框必须长高，且每行都完整显示（不截断中间行）
        let a = about();
        let ed = editor::Editor::from_text("第一行\n第二行\n第三行");
        let out = Screen {
            cols: 120, rows: 30, facts: &[], input: &ed, status: "就绪",
            awaiting_input: false, show_cursor: true,
            approval: None,
            about: Some(&a), trust: None,
            theme: theme::ThemeName::Neo, popup: None, preformatted: None,
            sidebar: false,
            view: None,
            diff_viewer: None,
            whichkey: None,
            display: ToolDisplay::default(),
            settings: None,
            settings_cursor: 0,
            settings_form: None, settings_confirm: None,
            settings_picker: None,
            appearance: appearance::Appearance::default(),
            custom_background: None,
        }
        .render();
        let text = plain(&out).join("\n");
        for needle in ["第一行", "第二行", "第三行"] {
            assert!(text.contains(needle), "多行内容应全部显示，缺 {needle}：{text}");
        }
    }

    #[test]
    fn chrome_height_is_dynamic_not_constant() {
        // 布局必须按输入行数算高度：用固定常量会让多行输入盖住正文
        assert_eq!(chrome_rows(1), 6, "单行时 chrome 应为 6 行");
        assert!(chrome_rows(3) > chrome_rows(1), "多行时 chrome 必须更高");
        // 上限：输入框不能吃掉整屏
        assert_eq!(chrome_rows(100), chrome_rows(MAX_INPUT_ROWS));
    }

    #[test]
    fn overlong_input_scrolls_to_the_cursor_line() {
        // 超过上限时显示**末尾**（光标总在可见区），并如实标注总行数
        let a = about();
        let text: String = (0..20).map(|i| format!("line{i}\n")).collect();
        let ed = editor::Editor::from_text(text.trim_end());
        let out = Screen {
            cols: 120, rows: 40, facts: &[], input: &ed, status: "",
            awaiting_input: false, show_cursor: true,
            approval: None,
            about: Some(&a), trust: None,
            theme: theme::ThemeName::Neo, popup: None, preformatted: None,
            sidebar: false,
            view: None,
            diff_viewer: None,
            whichkey: None,
            display: ToolDisplay::default(),
            settings: None,
            settings_cursor: 0,
            settings_form: None, settings_confirm: None,
            settings_picker: None,
            appearance: appearance::Appearance::default(),
            custom_background: None,
        }
        .render();
        let t = plain(&out).join("\n");
        assert!(t.contains("line19"), "应显示末尾（光标所在）行：{t}");
        assert!(t.contains("共 20 行"), "应如实标注总行数：{t}");
    }

    #[test]
    fn multiline_input_keeps_lines_within_width() {
        // 行宽不变量在多行输入下同样成立（否则折行会滚屏）
        let a = about();
        let long = "很长的中文输入行".repeat(10);
        let ed = editor::Editor::from_text(&format!("{long}\n{}", "x".repeat(300)));
        for cols in [96usize, 120, 200] {
            let out = Screen {
                cols, rows: 30, facts: &[], input: &ed, status: "",
                awaiting_input: false, show_cursor: true,
                approval: None,
                about: Some(&a), trust: None,
                theme: theme::ThemeName::Neo, popup: None, preformatted: None,
                sidebar: true,
                view: None,
                diff_viewer: None,
                whichkey: None,
                display: ToolDisplay::default(),
                settings: None,
                settings_cursor: 0,
            settings_form: None, settings_confirm: None,
                settings_picker: None,
                appearance: appearance::Appearance::default(),
                custom_background: None,
            }
            .render();
            for (i, l) in plain(&out).iter().enumerate() {
                let w = width::display_width(l);
                assert!(w <= cols, "{cols} 第 {i} 行宽 {w} 超宽：{l:?}");
            }
        }
    }

    #[test]
    fn cursor_is_positioned_at_the_editors_cursor_not_the_end() {
        // 光标必须落在编辑器的实际光标处。若仍按"文本末尾"算，
        // 在中间编辑时光标会跑到别处，用户以为敲不进去。
        let a = about();
        let mut ed = editor::Editor::from_text("abcdef");
        ed.move_home();
        ed.move_right();
        ed.move_right();
        let out = Screen {
            cols: 100, rows: 24, facts: &[], input: &ed, status: "",
            awaiting_input: false, show_cursor: true,
            approval: None,
            about: Some(&a), trust: None,
            theme: theme::ThemeName::Neo, popup: None, preformatted: None,
            sidebar: false,
            view: None,
            diff_viewer: None,
            whichkey: None,
            display: ToolDisplay::default(),
            settings: None,
            settings_cursor: 0,
            settings_form: None, settings_confirm: None,
            settings_picker: None,
            appearance: appearance::Appearance::default(),
            custom_background: None,
        }
        .render();
        // 提取光标定位序列 ESC[<row>;<col>H（渲染尾部还有 ?25h 之类，需精确匹配）
        let positions: Vec<usize> = out
            .split("\u{1b}[")
            .filter_map(|seg| {
                let digits: String =
                    seg.chars().take_while(|c| c.is_ascii_digit() || *c == ';').collect();
                if !seg[digits.len()..].starts_with('H') || !digits.contains(';') {
                    return None;
                }
                digits.split(';').nth(1).and_then(|s| s.parse::<usize>().ok())
            })
            .collect();
        let col = *positions.last().expect("应有光标定位序列 ESC[<row>;<col>H");
        // 光标应在第 3 个字符之后（col 2）→ 屏幕列 = 左边框位置 + 3
        assert!(col > 3, "光标应随编辑器光标移动（实际列 {col}）");
    }

    // ── 带超时读键（resize 轮询的基础）──────────────────────────────────

    #[test]
    fn timeout_read_returns_none_when_there_is_no_input() {
        // 这是"窗口 resize 后能重绘"的基础：无输入时读必须**返回**而不是阻塞，
        // 否则循环被钉死，resize 永远得不到处理（真机表现为画面错位/残留）。
        let mut empty: &[u8] = &[];
        assert_eq!(read_key_timeout(&mut empty, 1), None, "无输入应返回 None 而不是 Quit");
    }

    #[test]
    fn timeout_read_decodes_bytes_normally() {
        // 有输入时行为与阻塞读一致
        let mut enter: &[u8] = b"\r";
        assert_eq!(read_key_timeout(&mut enter, 1), Some(Key::Enter));
        let mut ctrlc: &[u8] = &[0x03];
        assert_eq!(read_key_timeout(&mut ctrlc, 1), Some(Key::Quit));
        let mut multi: &[u8] = "你".as_bytes();
        assert_eq!(read_key_timeout(&mut multi, 1), Some(Key::Char('你')));
    }

    #[test]
    fn trust_keys_are_handled_consistently() {
        // 抽出 handle_trust_key 的理由：让"等待输入"与"渲染"两条路径共用语义。
        // 这里断言每个键都被映射到明确的结论，不留模糊分支。
        let ws = std::path::Path::new("/tmp/does-not-matter");
        let mut tp = TrustPrompt::default();
        assert!(matches!(handle_trust_key(Key::Up, &mut tp, ws), TrustOutcome::Continue));
        assert_eq!(tp.selected, 0);
        assert!(matches!(handle_trust_key(Key::Down, &mut tp, ws), TrustOutcome::Continue));
        assert_eq!(tp.selected, 1);
        // Enter 跟随选中项：选中"否"时应退出而不是接受
        assert!(matches!(handle_trust_key(Key::Enter, &mut tp, ws), TrustOutcome::Quit));
        tp.selected = 0;
        assert!(matches!(handle_trust_key(Key::Quit, &mut tp, ws), TrustOutcome::Quit));
        assert!(matches!(handle_trust_key(Key::Char('n'), &mut tp, ws), TrustOutcome::Quit));
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
                Fact::ToolFinished {
                name: "bash".into(),
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
                truncated: false,
            },
            ];
            let out = Screen {
                cols, rows, facts: &facts, input: &editor::Editor::from_text("输入中文"), status: "就绪",
                awaiting_input: false, show_cursor: false, about: Some(&a), trust: None,
                approval: None,
                theme: theme::ThemeName::OpenCode, popup: None, preformatted: None, sidebar: false,
                view: None,
                diff_viewer: None,
                whichkey: None,
                display: ToolDisplay::default(),
                settings: None,
                settings_cursor: 0,
            settings_form: None, settings_confirm: None,
                settings_picker: None,
                appearance: appearance::Appearance::default(),
                custom_background: None,
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
        let ok = [Fact::ToolFinished {
                name: "bash".into(),
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
                truncated: false,
            }];
        let bad = [Fact::ToolFinished {
                name: "bash".into(),
                exit_code: 1,
                stdout: String::new(),
                stderr: String::new(),
                truncated: false,
            }];
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
        // 小终端必须降级而不是把内容裁掉（裁剪会从顶部裁，最难看）。
        // 默认样式是 Large，但终端不够宽时应退到 3 行小词标，而不是硬塞 6 行。
        let text = plain(&welcome(30, 10)).join("\n");
        assert!(text.contains("Neo"), "小终端仍要显示品牌：{text}");
        // 小词标用 ▀/█ 方块字符（3 行），大词标用 ╗╔ 等框线（6 行）。
        // 断言"没有大词标特有的框线字符"比"没有 █"更准确。
        assert!(
            !text.contains('╗') && !text.contains('╔'),
            "小终端不该硬塞 6 行大词标：{text}"
        );
        // 且确实退到了小词标（而不是什么都不画）
        assert!(text.contains('▀') || text.contains('█'), "应显示小词标：{text}");
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
        // 提示行只放最高频的几个键（完整键位在 /keys）
        assert!(text.contains("tab 补全"), "应提示补全：{text}");
        assert!(text.contains("ctrl+/"), "应提示 which-key（可发现性入口）：{text}");
        assert!(text.contains("@ 引用"), "应提示引用：{text}");
        assert!(text.contains("pgup"), "应提示滚动：{text}");
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
            cols: 100, rows: 30, facts: &[], input: &editor::Editor::new(), status: "",
            awaiting_input: false, show_cursor: false,
            approval: None,
            about: Some(&a), trust: Some(&TrustPrompt::default()),
            theme: theme::ThemeName::OpenCode, popup: None, preformatted: None, sidebar: false,
            view: None,
            diff_viewer: None,
            whichkey: None,
            display: ToolDisplay::default(),
            settings: None,
            settings_cursor: 0,
            settings_form: None, settings_confirm: None,
            settings_picker: None,
            appearance: appearance::Appearance::default(),
            custom_background: None,
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
                cols: 100, rows: 30, facts: &[], input: &editor::Editor::new(), status: "",
                awaiting_input: false, show_cursor: false,
                approval: None,
                about: Some(&a), trust: Some(&TrustPrompt { selected: sel }),
                theme: theme::ThemeName::OpenCode, popup: None, preformatted: None, sidebar: false,
                view: None,
                diff_viewer: None,
                whichkey: None,
                display: ToolDisplay::default(),
                settings: None,
                settings_cursor: 0,
            settings_form: None, settings_confirm: None,
                settings_picker: None,
                appearance: appearance::Appearance::default(),
                custom_background: None,
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
            EventMsg::ToolCallEnd { id: "c".into(), exit_code: 0, stdout: String::new(), stderr: String::new(), truncated: false },
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
    fn approval_prompt_renders_a_modal_with_choices() {
        // 审批必须是**对话框**（用户明确反馈输入框里回 y 不像 opencode/mimo）。
        // 断言：卡片出现、三个选项都在、"批的是什么"也在。
        let ed = editor::Editor::new();
        let ap = ApprovalPrompt {
            detail: "写入类调用需确认".into(),
            call: "bash rm -rf /tmp/x".into(),
            diff: Some(("a.txt".into(), "@@ -1 +1 @@\n-old\n+new".into())),
            selected: 1,
        };
        let out = Screen {
            cols: 120, rows: 40, facts: &[], input: &ed, status: "",
            awaiting_input: true, show_cursor: false,
            approval: Some(&ap),
            about: Some(&about()), trust: None,
            theme: theme::ThemeName::Neo, popup: None, preformatted: None,
            sidebar: false, view: None, diff_viewer: None, whichkey: None,
            display: ToolDisplay::default(), settings: None, settings_cursor: 0,
            settings_picker: None, settings_form: None, settings_confirm: None,
            appearance: appearance::Appearance::default(), custom_background: None,
        }
        .render();
        let text = plain(&out).join("
");
        assert!(text.contains("需要审批"), "应有模态标题：{text}");
        assert!(text.contains("bash rm -rf /tmp/x"), "应说明批的是什么");
        for c in ApprovalPrompt::CHOICES {
            assert!(text.contains(c), "缺选项 {c}");
        }
        // 选中项要有 ❯ 标记
        assert!(text.contains("❯ 总是允许这类"), "选中项应有标记");
    }

    #[test]
    fn parses_approval_answers_and_rejects_noise() {
        use ApprovalAnswer::*;
        for yes in ["y", "Y", "yes", "YES", " y "] {
            assert_eq!(parse_approval_answer(yes), Some(Allow), "{yes} 应判为批准");
        }
        // `a` = 总是允许这类：没有它用户会被反复问（真实反馈：连按十几次 y）
        for always in ["a", "A", "always", "all"] {
            assert_eq!(
                parse_approval_answer(always),
                Some(AllowAlways),
                "{always} 应判为总是允许"
            );
        }
        for no in ["n", "N", "no"] {
            assert_eq!(parse_approval_answer(no), Some(Deny), "{no} 应判为拒绝");
        }
        assert_eq!(parse_approval_answer("maybe"), None);
        assert_eq!(parse_approval_answer(""), None);
    }

    #[test]
    fn tui_host_reports_facts_from_the_shared_extractor() {
        let mut h = TuiFacts::new();
        h.consume(&EventMsg::TurnStarted { turn_id: "t".into() }).unwrap();
        h.consume(&EventMsg::AgentMessageDone { text: "hi".into() }).unwrap();
        h.consume(&EventMsg::ToolCallBegin { id: "c".into(), name: "bash".into(), arguments: serde_json::Value::Null }).unwrap();
        h.consume(&EventMsg::ToolCallEnd { id: "c".into(), exit_code: 0, stdout: String::new(), stderr: String::new(), truncated: false }).unwrap();
        assert_eq!(
            h.facts(),
            vec![
                Fact::AssistantSaid("hi".into()),
                Fact::ToolFinished {
                name: "bash".into(),
                exit_code: 0,
                stdout: String::new(),
                stderr: String::new(),
                truncated: false,
            },
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
                    cols: 80, rows: 20, facts: &[], input: &editor::Editor::new(), status: "",
                    awaiting_input: awaiting, show_cursor: false,
                    approval: None,
                    about: Some(&a), trust: None,
                    theme: theme::ThemeName::OpenCode, popup: None, preformatted: None,
                    sidebar: false,
                    view: None,
                    diff_viewer: None,
                    whichkey: None,
                    display: ToolDisplay::default(),
                    settings: None,
                    settings_cursor: 0,
            settings_form: None, settings_confirm: None,
                    settings_picker: None,
                    appearance: appearance::Appearance::default(),
                    custom_background: None,
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
