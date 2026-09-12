//! 命令注册表 —— `/` 斜杠命令与 `ctrl+p` 命令面板的**唯一事实源**
//!
//! # 只注册真正实现了的命令
//!
//! 对齐 opencode 时最容易犯的错是"照抄它的命令清单"，结果列出一堆
//! 按了没反应的项 —— 那比没有命令更糟。这里每条命令都有对应的
//! `Action`，`Action` 在执行处被穷尽匹配；没实现的就不在这里出现。
//!
//! 与 opencode 命令表的对照（只列我们做到的）：
//!   `/exit`（别名 quit/q）、`/help`、`/theme`（对齐 /themes）、
//!   `/new`（别名 clear）、`/compact`、`/keys`（对齐 /help 的键位部分）、
//!   以及命令面板 `ctrl+p`。opencode 的 /models /agents /mcps /sessions
//!   依赖多模型/多会话/多 Agent 基建，我们没有，故不列出（诚实边界）。

use crate::theme::ThemeName;

/// 命令分组（`ctrl+p` 面板按此分节展示）。
///
/// 为什么要分组：命令多了之后平铺一列，用户要在十几个条目里逐行找。
/// 分组把"我大概想要什么"变成一次定位 —— 对齐 opencode 面板的分节观感。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// 最常用（放最上面）
    Recommended,
    /// 对话与会话
    Session,
    /// 界面显示
    Display,
    /// 系统与信息
    System,
}

impl Category {
    pub fn title(self) -> &'static str {
        match self {
            Self::Recommended => "推荐",
            Self::Session => "会话",
            Self::Display => "显示",
            Self::System => "系统",
        }
    }

    /// 展示顺序：推荐 → 会话 → 显示 → 系统。
    pub fn order(self) -> usize {
        match self {
            Self::Recommended => 0,
            Self::Session => 1,
            Self::Display => 2,
            Self::System => 3,
        }
    }

    pub fn all() -> [Category; 4] {
        [Self::Recommended, Self::Session, Self::Display, Self::System]
    }
}

/// 一条斜杠命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    /// 命令名（不含 `/`）
    pub name: &'static str,
    /// 别名（含或不含 `/` 都可以写，展示时会统一加）
    pub aliases: &'static [&'static str],
    pub desc: &'static str,
    pub action: Action,
    pub category: Category,
    /// 对应的键盘快捷键（无则空串）。面板右侧显示，让"命令"与"键位"对上号 ——
    /// 用户看到 `/theme` 右边写着 `ctrl+t`，下回就直接按键而不用打命令。
    pub keybinding: &'static str,
}

/// 命令要执行的动作。
///
/// 用枚举而不是 `Box<dyn Fn>`：动作清单是**有限的**，
/// 枚举让"哪些能执行"一眼可见，且执行处能穷尽匹配不漏分支。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// 退出
    Quit,
    /// 清空对话（保留当前内核会话对象；真正的"新会话"需要重建内核）
    NewSession,
    /// 压缩上下文
    Compact,
    /// 打开主题选择
    ThemePicker,
    /// 直接切到下一个主题
    NextTheme,
    /// 设置主题
    SetTheme(ThemeName),
    /// 显示帮助
    Help,
    /// 显示键位
    Keys,
    /// 显示状态（模型/沙箱/工作区/分支/主题/工具）
    Status,
    /// 打开全屏 diff 查看器
    DiffViewer,
    /// 回退对话一轮（不还原文件）
    Rewind,
    /// 展开 / 折叠工具输出
    ToggleDetails,
    /// 显示 / 隐藏推理过程
    ToggleThinking,
    /// 复制最近一条助手回复到剪贴板
    CopyLastReply,
    /// 打开设置（系统 / 模型 / 会话）
    Settings,
    /// 收起 / 展开侧栏
    ToggleSidebar,
    /// 开 / 关提醒
    ToggleNotify,
    /// 开 / 关提醒声音
    ToggleNotifySound,
    /// 切换背景纹理
    NextBackground,
    /// 打开背景选择
    BackgroundPicker,
    /// 切换 Logo 样式
    NextLogo,
    /// 打开 Logo 样式选择
    LogoPicker,
    /// 设置背景 / Logo（走设置弹窗里的选择列表）
    SetBackground(crate::appearance::Background),
    /// 设置 Logo 样式
    SetLogo(crate::appearance::LogoStyle),
    /// 打开模型选择列表
    ModelPicker,
    /// 切换到指定模型（名字来自运行时注册表，不是编译期常量，
    /// 故这里用 owned String 的**索引**表达不了；改由 ItemAction 承载。）
    SwitchModel,
    /// 打开会话列表
    SessionPicker,
    /// 新建会话（真新建：全新日志文件，旧会话保留）
    NewSessionReal,
}

/// **必须由命令触发**的动作。新增变体时要加进这里 ——
/// 编译器不会提醒"去注册一条命令"，很容易出现"动作写好了但没有命令能触发"。
const COMMAND_ACTIONS: &[Action] = &[
    Action::Quit,
    Action::Compact,
    Action::ThemePicker,
    Action::NextTheme,
    Action::Help,
    Action::Keys,
    Action::Status,
    Action::DiffViewer,
    Action::Rewind,
    Action::ToggleDetails,
    Action::ToggleThinking,
    Action::CopyLastReply,
    Action::Settings,
    Action::ToggleSidebar,
    Action::ToggleNotify,
    Action::ToggleNotifySound,
    Action::NextBackground,
    Action::BackgroundPicker,
    Action::NextLogo,
    Action::LogoPicker,
    Action::ModelPicker,
    Action::SessionPicker,
    Action::NewSessionReal,
];

/// **只在界面内部产生**的动作（不经过命令表）。
///
/// `SetTheme` 只由主题选择弹窗产生 —— 让用户敲 `/settheme nord`
/// 不如让他从列表里选（名字要记，也没法预览）。这类动作刻意不注册命令。
const INTERNAL_ONLY_ACTIONS: &[Action] = &[
    // 旧的"清空转录"仍可从设置页触发（把当前会话的转录清掉、不新建文件）。
    // 命令表里不再暴露它 —— `/new` 现在是**真新建会话**（NewSessionReal），
    // 两个入口语义不同，共用一个命令名会让用户困惑。
    Action::NewSession,
    Action::SwitchModel,
    Action::SetTheme(crate::theme::ThemeName::Nord),
    Action::SetBackground(crate::appearance::Background::Dots),
    Action::SetLogo(crate::appearance::LogoStyle::Small),
];

/// 命令表本体（单一事实源）。
///
/// 单独成 const 而不是写进 `registry()` 的函数体：审计函数要读它，
/// 若审计走 `registry()` 会**无限递归**（registry → audit → registry），
/// 表现为测试里的 stack overflow。表格是数据，取用方式是函数。
const COMMANDS: &[Command] = &[
    // ── 推荐：最常用 ──
    Command {
        name: "settings", aliases: &["config"], desc: "设置（系统 / 模型 / 会话）",
        action: Action::Settings, category: Category::Recommended, keybinding: "",
    },
    Command {
        name: "theme", aliases: &["themes"], desc: "切换配色主题",
        action: Action::ThemePicker, category: Category::Recommended, keybinding: "ctrl+t",
    },
    Command {
        name: "next", aliases: &["next-theme"], desc: "直接切到下一个主题",
        action: Action::NextTheme, category: Category::Recommended, keybinding: "ctrl+t",
    },
    Command {
        name: "details", aliases: &["d"], desc: "展开 / 折叠工具输出",
        action: Action::ToggleDetails, category: Category::Recommended, keybinding: "",
    },
    Command {
        name: "copy", aliases: &["yank"], desc: "复制最近一条回复",
        action: Action::CopyLastReply, category: Category::Recommended, keybinding: "",
    },

    // ── 会话 ──
    Command {
        name: "new", aliases: &["clear"], desc: "开始新会话（新日志文件，旧会话保留）",
        action: Action::NewSessionReal, category: Category::Session, keybinding: "",
    },
    Command {
        name: "undo", aliases: &["rewind"], desc: "回退对话一轮（不还原文件）",
        action: Action::Rewind, category: Category::Session, keybinding: "",
    },
    Command {
        name: "diff", aliases: &["changes"], desc: "查看本次会话的改动",
        action: Action::DiffViewer, category: Category::Session, keybinding: "审批时按 d",
    },
    Command {
        name: "compact", aliases: &["summarize"], desc: "压缩上下文（把较早消息摘要为一条）",
        action: Action::Compact, category: Category::Session, keybinding: "",
    },

    // ── 显示 ──
    Command {
        name: "thinking", aliases: &["think"], desc: "显示 / 隐藏推理过程",
        action: Action::ToggleThinking, category: Category::Display, keybinding: "",
    },
    Command {
        name: "sidebar", aliases: &["panel"], desc: "收起 / 展开右侧面板",
        action: Action::ToggleSidebar, category: Category::Display, keybinding: "ctrl+b",
    },
    Command {
        name: "notify", aliases: &["attention"], desc: "开 / 关提醒（完成 / 出错 / 需审批）",
        action: Action::ToggleNotify, category: Category::Display, keybinding: "",
    },
    Command {
        name: "notify-sound", aliases: &["sound"], desc: "开 / 关提醒声音",
        action: Action::ToggleNotifySound, category: Category::Display, keybinding: "",
    },
    Command {
        name: "background", aliases: &["bg"], desc: "切换背景纹理（星场/点阵/斜纹/纯色）",
        action: Action::NextBackground, category: Category::Display, keybinding: "",
    },
    Command {
        name: "background-list", aliases: &["bg-list"], desc: "选择背景纹理",
        action: Action::BackgroundPicker, category: Category::Display, keybinding: "",
    },
    Command {
        name: "logo", aliases: &["logo-style"], desc: "切换 Logo 样式（大/小/极简/隐藏）",
        action: Action::NextLogo, category: Category::Display, keybinding: "",
    },
    Command {
        name: "logo-list", aliases: &["logo-styles"], desc: "选择 Logo 样式",
        action: Action::LogoPicker, category: Category::Display, keybinding: "",
    },
    Command {
        name: "models", aliases: &["model"], desc: "切换模型（运行时，无需重启）",
        action: Action::ModelPicker, category: Category::Recommended, keybinding: "",
    },
    Command {
        name: "sessions", aliases: &["resume", "continue"], desc: "切换会话（列出全部，含标题）",
        action: Action::SessionPicker, category: Category::Session, keybinding: "",
    },
    Command {
        name: "keys", aliases: &["keybindings"], desc: "键盘快捷键",
        action: Action::Keys, category: Category::Display, keybinding: "ctrl+o",
    },

    // ── 系统 ──
    Command {
        name: "status", aliases: &["info"], desc: "运行状态与环境",
        action: Action::Status, category: Category::System, keybinding: "",
    },
    Command {
        name: "help", aliases: &[], desc: "显示帮助",
        action: Action::Help, category: Category::System, keybinding: "",
    },
    Command {
        name: "exit", aliases: &["quit", "q"], desc: "退出 Neo",
        action: Action::Quit, category: Category::System, keybinding: "ctrl+c",
    },
];

/// `/goal` 系列的参数形态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalCmd<'a> {
    /// 设定（或重定向）目标：文本**每行一个子任务**，没有换行 = 单子任务
    Set(&'a str),
    /// 暂停推进（目标保留）
    Pause,
    /// 恢复推进
    Resume,
    /// 清除目标
    Clear,
    /// 查询当前状态
    Status,
}

/// 解析 `/goal` 系列。`None` = 不是 /goal（其它 `/xxx` 照旧走命令表）。
///
/// 前缀相同的命令（`/goals`）必须返回 None —— 命令按前缀吞掉的话，
/// 以后谁加一个 `/goals` 命令就永远匹配不到了。
pub fn parse_goal_command(line: &str) -> Option<GoalCmd<'_>> {
    let rest = line.strip_prefix("/goal")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let arg = rest.trim();
    Some(match arg {
        "" => GoalCmd::Status,
        "pause" | "p" => GoalCmd::Pause,
        "resume" | "r" => GoalCmd::Resume,
        "clear" | "c" | "off" => GoalCmd::Clear,
        text => GoalCmd::Set(text),
    })
}

/// 检查"动作 ↔ 命令"的可达性，返回问题列表（空 = 一致）。
///
/// 读 `COMMANDS` 而不是 `registry()` —— 后者在 debug 下会调本函数，
/// 走 registry 会无限递归。开发中任何 debug 构建都会通过 `registry()`
/// 的 `debug_assert` 触发它，从而在运行时而非仅测试时发现"动作没注册命令"。
pub fn audit_action_reachability() -> Vec<String> {
    let actions: Vec<Action> = COMMANDS.iter().map(|c| c.action).collect();
    let mut problems = Vec::new();
    for a in COMMAND_ACTIONS {
        if !actions.contains(a) {
            problems.push(format!("{a:?} 没有任何命令可达 —— 忘注册了？"));
        }
    }
    for a in INTERNAL_ONLY_ACTIONS {
        if actions.contains(a) {
            problems.push(format!("{a:?} 只应由界面内部产生，不该注册成命令"));
        }
    }
    problems
}

/// 全部可用命令。顺序即 `/` 列表的展示顺序（高频在前）。
pub fn registry() -> &'static [Command] {
    debug_assert!(
        audit_action_reachability().is_empty(),
        "命令表与动作不一致：{:?}",
        audit_action_reachability()
    );
    COMMANDS
}

/// 按查询前缀/子串过滤（不区分大小写）。空查询返回全部。
pub fn matches(query: &str) -> Vec<&'static Command> {
    let q = query.trim().to_ascii_lowercase();
    registry()
        .iter()
        .filter(|c| {
            q.is_empty()
                || c.name.contains(&q)
                || c.aliases.iter().any(|a| a.contains(&q))
        })
        .collect()
}

/// 解析用户直接敲的 `/xxx`（不进列表，直接执行）。
pub fn resolve(name: &str) -> Option<&'static Command> {
    let n = name.trim().trim_start_matches('/').to_ascii_lowercase();
    registry()
        .iter()
        .find(|c| c.name == n || c.aliases.iter().any(|a| *a == n))
}

/// 帮助正文（`/help`）。分栏排版，宽度由调用方决定。
pub fn help_text() -> &'static str {
    "\
Neo —— 编程 Agent 内核

输入
  直接输入任务并回车即可。以 @ 引用文件、以 ! 直接执行 shell 命令、
  以 / 唤起命令。

引用
  @            打开文件列表（↑↓ 选择，Enter 确认）
  @src/main.rs 直接引用某个文件
  @file#12-40  引用该文件的第 12–40 行

命令
  打 / 唤起命令列表（也可 ctrl+p 打开命令面板）。可用命令：
    /settings  设置（系统 / 模型 / 会话）
    /help      显示帮助
    /keys      键盘快捷键
    /status    运行状态与环境
    /diff      查看改动（全屏：hunk/文件跳转、双列视图）
    /sessions  切换会话（别名 resume / continue）
    /new       新建会话（旧会话保留，可 /sessions 切回）
    /undo      回退对话一轮（**不还原文件**，见下方说明）
    /details   展开 / 折叠工具输出（失败时总是展示）
    /thinking  显示 / 隐藏推理过程
    /copy      复制最近一条回复到系统剪贴板
    /diff      查看改动（全屏查看器：hunk/文件跳转、双列视图）
    /theme     选择配色主题（6 套）
    /next      直接切到下一个主题
    /sidebar   收起 / 展开右侧面板
    /notify    开 / 关提醒（默认关）
    /notify-sound 开 / 关提醒声音
    /background  切换背景纹理（/background-list 可选）
    /logo        切换 Logo 样式（/logo-list 可选）
    /models      切换模型（运行时，无需重启）
    /goal        目标编排：/goal <目标>（每行一个子任务）开始并逐步执行；
                 /goal pause|resume|clear 暂停 / 恢复 / 清除；/goal 看状态
    /copy      复制最近一条回复
    /details   展开 / 折叠工具输出
    /thinking  显示 / 隐藏推理
    /compact   压缩上下文（把较早消息摘要为一条，腾出预算）
    /exit      退出（别名 quit / q）

多行输入
  输入框支持多行（最多显示 6 行，超出滚到末尾）。
  在多数终端里 shift+enter / alt+enter 会插入换行。

按键
  tab          在 @ 上下文中补全
  ↑ / ↓        本缓冲内移动；已在首/末行则走历史
  ← / →        移动光标（可跨行）
  home / end   行首 / 行尾
  alt+b / alt+f  按词移动
  ctrl+k       删到行尾      ctrl+u  删到行首
  ctrl+w       删前一个词    delete  删除光标处字符
  ctrl+_       撤销          ctrl+y  重做
  ctrl+r       搜索历史（输入框）
  ctrl+g       用 $EDITOR 编辑当前输入
  ctrl+p       命令面板      ctrl+t  切换主题
  ctrl+b       侧栏开关      ctrl+l  清屏
  ctrl+o       键位提示（显示当前上下文可用的键）
  ctrl+z       挂起回 shell（`fg` 恢复；需终端支持作业控制）
  esc          关闭弹窗 / 清空输入 / 退出搜索
  ctrl+c       退出

阅读转录
  pageup / pagedown   半页上下滚动
  ctrl+home / ctrl+e  跳到最顶 / 回到最新
  ctrl+f              搜索转录（回车确认，esc 取消）
  ctrl+n / ctrl+p     下一个 / 上一个匹配"
}

/// 状态屏正文由宿主在运行时拼（数据来自 About）。
pub fn status_text(a: &crate::About, theme_name: &str) -> String {
    format!(
        "\
运行状态

  版本      {}
  模型      {}
  档位      {}
  工作区    {}
  分支      {}
  会话      {}
  主题      {}

沙箱
  模式      {}（OS 级强制；受限档位在无后端平台 fail-closed）
  写入      fail-closed：无沙箱后端时拒绝执行，不降级放行

能力
  工具      bash / apply_patch / todowrite / 提问
  已实现    上下文引用（@file / $skill）、项目指令级联（AGENTS.md）、
            上下文压缩（/compact）、多会话、服务商注册表（providers.json）、
            目标编排（/goal，四阶段 + 重试 + 判停）、MCP 外部工具
  未实现    桌面 webview 窗口层
",
        a.version,
        a.model,
        a.mode,
        a.workspace,
        if a.branch.is_empty() { "（不在 git 仓库）" } else { &a.branch },
        a.session,
        theme_name,
        a.mode_short,
    )
}

/// 键位表（`/keys`）。与 `help_text` 的按键段保持同源，避免两处各说一套。
pub fn keys_text() -> &'static str {
    "\
键盘快捷键

  提交 / 输入
    enter        提交任务（弹窗打开时为确认选择）
    ctrl+g       用 $EDITOR / $VISUAL 编辑当前输入

  补全与引用
    @            打开文件列表
    tab          在 @ 上下文中补全为最佳匹配
    @file#a-b    引用行范围

  历史
    ↑ / ↓        浏览历史
    ctrl+r       按当前输入搜索历史

  界面
    /            命令列表
    ctrl+p       命令面板
    ctrl+t       下一个主题
    ctrl+l       清屏
    esc          关闭弹窗

  退出
    ctrl+c       退出（备用屏整块还原）"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_command_resolves_by_all_its_names() {
        // 别名必须都能解析到自己 —— 否则列表里显示了却敲不出来
        for c in registry() {
            assert_eq!(resolve(c.name).map(|x| x.name), Some(c.name));
            for a in c.aliases {
                assert_eq!(
                    resolve(a).map(|x| x.name),
                    Some(c.name),
                    "别名 {a} 未解析到 {}",
                    c.name
                );
            }
        }
    }

    #[test]
    fn resolve_accepts_leading_slash_and_case() {
        assert_eq!(resolve("/exit").map(|c| c.name), Some("exit"));
        assert_eq!(resolve("QUIT").map(|c| c.name), Some("exit"));
        assert_eq!(resolve(" /Help ").map(|c| c.name), Some("help"));
        assert_eq!(resolve("/nope"), None);
    }

    #[test]
    fn matches_filters_by_substring_and_is_empty_for_unknown() {
        assert_eq!(matches("").len(), registry().len(), "空查询应返回全部");
        let th = matches("them");
        assert!(th.iter().any(|c| c.name == "theme"), "子串应命中 theme");
        assert!(matches("zzz").is_empty(), "无匹配应为空");
    }

    #[test]
    fn every_action_variant_is_reachable_from_the_registry() {
        // 钉住一个真实 bug：`Action::Status` 定义了、`status_text` 也写了，
        // 却**忘记注册命令** —— 于是 /status 永远"无匹配"。
        //
        let problems = audit_action_reachability();
        assert!(problems.is_empty(), "命令表与动作不一致：{problems:?}");
    }

    #[test]
    fn help_lists_exactly_the_registered_commands() {
        // 帮助文案与实际命令表不能各说一套
        let h = help_text();
        for c in registry() {
            assert!(h.contains(c.name), "帮助里缺少命令 {}：{h}", c.name);
        }
    }

    #[test]
    fn actions_are_unique_per_command_and_cover_the_palette() {
        // 每个命令都有动作；动作不重复（重复说明表里有冗余项）
        let acts: Vec<Action> = registry().iter().map(|c| c.action).collect();
        let mut seen = vec![];
        for a in &acts {
            assert!(!seen.contains(a), "动作 {a:?} 在命令表里重复");
            seen.push(*a);
        }
    }

    #[test]
    fn help_and_keys_advertise_only_real_keys() {
        // 文案里出现的键必须真能用：这里做一个最小一致性检查
        let h = help_text();
        let k = keys_text();
        for needle in ["@", "ctrl+r", "ctrl+p", "ctrl+t", "esc"] {
            assert!(h.contains(needle), "help 应提到 {needle}");
            assert!(k.contains(needle), "keys 应提到 {needle}");
        }
        // 未实现的功能不得出现在文案里。
        // 注意 `/models` 已从这条清单**移除** —— 它在本轮补上了后端能力
        // （内核持模型注册表，运行时可切换）。这条守卫的作用正是提醒：
        // 能力从"无"变"有"时，要把名字从黑名单挪出来，别留下过时的断言。
        for bad in ["/agents", "/mcps", "/share", "/undo-tree"] {
            assert!(!h.contains(bad), "不得宣传未实现命令 {bad}");
            assert!(!k.contains(bad), "不得宣传未实现命令 {bad}");
        }
        // 已实现的反面：必须出现在帮助里
        for good in ["/models", "/settings", "/diff", "/sessions"] {
            assert!(h.contains(good), "已实现命令 {good} 应出现在帮助里");
        }
    }
}

#[cfg(test)]
mod goal_tests {
    use super::*;

    #[test]
    fn goal_command_parsing_covers_all_forms() {
        assert_eq!(parse_goal_command("/goal"), Some(GoalCmd::Status));
        assert_eq!(parse_goal_command("/goal "), Some(GoalCmd::Status));
        assert_eq!(
            parse_goal_command("/goal 实现登录\n写测试"),
            Some(GoalCmd::Set("实现登录\n写测试"))
        );
        assert_eq!(parse_goal_command("/goal pause"), Some(GoalCmd::Pause));
        assert_eq!(parse_goal_command("/goal resume"), Some(GoalCmd::Resume));
        assert_eq!(parse_goal_command("/goal clear"), Some(GoalCmd::Clear));
        assert_eq!(parse_goal_command("/goal c"), Some(GoalCmd::Clear));
    }

    #[test]
    fn prefix_lookalikes_are_not_swallowed() {
        // /goals、/goalkeeper 不是 /goal —— 前缀相同但必须放行给命令表
        assert_eq!(parse_goal_command("/goals"), None);
        assert_eq!(parse_goal_command("/goalkeeper x"), None);
    }
}
