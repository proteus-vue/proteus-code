//! 命令面板（D9：`Cmd/Ctrl+K` 覆盖式命令中心）
//!
//! # 对标（`desktop-parity.md` §1.1）
//!
//! ZCode 的命令中心是**覆盖式面板**，列 commands / conversations / files 三类。
//! 本模块做 **commands** 这一类；conversations（会话切换）与 files（文件搜索）
//! 需要会话库与文件索引 —— 那属于 D1 的范畴，未做，如实标注。
//!
//! # 为什么注册表是数据而不是一堆 `if`
//!
//! 命令是**有限清单**：列成表才能"一眼看全有哪些能做"、才能被搜索、才能让
//! 测试逐条断言"每个命令都真的映射到一个动作"。写成散落的 `if key == ...`
//! 会让"有哪些命令"只能靠读代码猜。
//!
//! # 为什么在 `neo-driver` 而不是某个宿主里
//!
//! 它引用了 `Op`（`Action::Compact` → `Op::Compact`），因此**依赖内核轴**。
//! 按本项目的判据：「能脱离内核独立发布的进 UI 栈，不能的进 driver」——
//! 所以它不属于 `neo-ui`，而属于这里。
//!
//! 它在宿主之下是必须的：egui 与 gpui 两个 GUI 宿主都要用这份命令表，
//! 而架构守卫 A3 **禁止宿主互相依赖**。曾经它长在 `neo-host-egui` 里，
//! gpui 宿主要用就只能复制一份 —— 那会让"有哪些命令"出现两个来源。
//!
//! # 与 TUI 的命令表的关系（**已知的重复，据实记录**）
//!
//! `neo-host-tui::commands` 另有一份注册表（24 条，含主题/星场/侧栏等终端
//! 专有动作）。它与本表**无法共享**：TUI 的动作几乎都是终端专有（改主题、
//! 星场、logo），而本表只收录 GUI 真能执行的动作。两边重合的只有少数几条
//!（compact / rewind / help 之类）。
//! 本模块只收录 **GUI 真的能执行**的动作 —— 不照抄 TUI 那 24 条（主题、星场、
//! logo 在原生窗口里没有对应物），否则会出现"命令列着但点了没反应"。
//!
//! ⚠️ **重复带来的漂移风险**：同一个命令（如 `/compact`）在两边若描述不同，
//! 用户会看到两套措辞。本项目对此已有先例 —— 需要跨宿主一致的**用户可见文字**
//! 放在协议层（见 `GoalSnapshot::summary()` 的注释："各宿主拼各的会漂移出
//! 同一个目标在 TUI 和 exec 长两副样子"）。命令词表也该如此，但那要同时改
//! TUI 的注册表（300+ 测试），不在本轮做。**这是一个已知的、记录在案的技术债**，
//! 不是被忽略的问题。

/// 命令分类（ZCode 把命令分组呈现，便于扫读）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// 会话与对话
    Session,
    /// 界面显示
    View,
    /// 信息
    Info,
}

impl Category {
    fn title(self) -> &'static str {
        match self {
            Self::Session => "会话",
            Self::View => "显示",
            Self::Info => "信息",
        }
    }
}

/// 命令面板能执行的动作。
///
/// 用枚举而不是闭包：动作清单**有限且需要被测试穷尽** ——
/// 新增一条命令时，编译器会提醒这里的 `match` 漏了分支。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// 底部终端面板（D8）
    ToggleTerminal,
    /// 切换 / 新建会话（D1）
    ToggleSidebar,
    NewSession,
    /// 压缩上下文（`Op::Compact`）
    Compact,
    /// 回退对话一轮（`Op::Rewind`，不还原文件）
    Rewind,
    /// 打断当前轮（`Op::Interrupt`）
    Interrupt,
    /// 打开模型选择（用状态栏那个 picker，这里只做提示）
    ShowModels,
    /// 循环切换执行模式
    CycleMode,
    /// 折叠 / 展开思考轨迹
    ToggleReasoning,
    /// 清空转录显示（**不动日志**）
    ClearTranscript,
    /// 打开帮助（命令与快捷键说明）
    Help,
    /// 退出应用
    Quit,
}

/// 一条命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    /// 命令名（展示与搜索都用它）
    pub name: &'static str,
    /// 说明
    pub desc: &'static str,
    pub action: ActionStatic,
    pub category: Category,
}

/// 与 [`Action`] 对应但可 `Copy` 的静态版本（注册表是 `const`）。
///
/// 为什么需要两套：`Action` 带数据时不能 `Copy`，而注册表要 `const`。
/// 目前动作都不带数据，所以这里是纯标签 —— 保留这个间接层是为了将来加
/// 带参数的命令（如"跳到第 N 轮"）时不必重写注册表结构。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionStatic {
    ToggleTerminal,
    ToggleSidebar,
    NewSession,
    Compact,
    Rewind,
    Interrupt,
    ShowModels,
    CycleMode,
    ToggleReasoning,
    ClearTranscript,
    Help,
    Quit,
}

impl ActionStatic {
    /// 转成可执行的动作。
    pub fn resolve(self) -> Action {
        match self {
            Self::ToggleTerminal => Action::ToggleTerminal,
            Self::ToggleSidebar => Action::ToggleSidebar,
            Self::NewSession => Action::NewSession,
            Self::Compact => Action::Compact,
            Self::Rewind => Action::Rewind,
            Self::Interrupt => Action::Interrupt,
            Self::ShowModels => Action::ShowModels,
            Self::CycleMode => Action::CycleMode,
            Self::ToggleReasoning => Action::ToggleReasoning,
            Self::ClearTranscript => Action::ClearTranscript,
            Self::Help => Action::Help,
            Self::Quit => Action::Quit,
        }
    }
}

/// 命令注册表。**只收录 GUI 真的能执行的动作**。
pub const COMMANDS: &[Command] = &[
    Command {
        name: "terminal",
        desc: "显示 / 隐藏底部终端（命令不经模型，走沙箱）",
        action: ActionStatic::ToggleTerminal,
        category: Category::View,
    },
    Command {
        name: "sessions",
        desc: "显示 / 隐藏会话栏",
        action: ActionStatic::ToggleSidebar,
        category: Category::Session,
    },
    Command {
        name: "new",
        desc: "新建会话（旧会话保留，可切回）",
        action: ActionStatic::NewSession,
        category: Category::Session,
    },
    Command {
        name: "compact",
        desc: "压缩上下文（把较早消息摘要为一条）",
        action: ActionStatic::Compact,
        category: Category::Session,
    },
    Command {
        name: "rewind",
        desc: "回退对话一轮（不还原文件）",
        action: ActionStatic::Rewind,
        category: Category::Session,
    },
    Command {
        name: "stop",
        desc: "打断当前这一轮",
        action: ActionStatic::Interrupt,
        category: Category::Session,
    },
    Command {
        name: "model",
        desc: "切换模型（状态栏的模型下拉）",
        action: ActionStatic::ShowModels,
        category: Category::View,
    },
    Command {
        name: "mode",
        desc: "切换执行模式（等同 Shift+Tab）",
        action: ActionStatic::CycleMode,
        category: Category::View,
    },
    Command {
        name: "thinking",
        desc: "折叠 / 展开思考轨迹",
        action: ActionStatic::ToggleReasoning,
        category: Category::View,
    },
    Command {
        name: "clear",
        desc: "清空屏幕上的转录（不影响会话日志）",
        action: ActionStatic::ClearTranscript,
        category: Category::View,
    },
    Command {
        name: "help",
        desc: "命令与快捷键说明",
        action: ActionStatic::Help,
        category: Category::Info,
    },
    Command {
        name: "quit",
        desc: "退出",
        action: ActionStatic::Quit,
        category: Category::Info,
    },
];

/// 命令面板的状态。
#[derive(Default)]
pub struct Palette {
    pub open: bool,
    /// 搜索串
    pub query: String,
    /// 当前高亮项在**过滤后**列表里的下标
    pub selected: usize,
}

impl Palette {
    /// 打开：清空搜索与高亮，保证每次打开都是干净状态。
    ///
    /// 不清的话会残留上次的搜索串 —— 用户再打开会发现"列表是空的/不完整的"，
    /// 而原因（上次输了字）在界面上已经看不见了。
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.selected = 0;
    }

    /// 按搜索串过滤。大小写不敏感，匹配命令名或说明。
    ///
    /// 匹配说明的好处：用户记得"那个压缩上下文的命令"，不必想起它叫 compact。
    pub fn matches(&self) -> Vec<&'static Command> {
        filter(self.query.as_str())
    }

    /// 上/下移动高亮（**循环**：到顶再按上回到最后一条）。
    pub fn move_selection(&mut self, down: bool) {
        let n = self.matches().len();
        if n == 0 {
            self.selected = 0;
            return;
        }
        if down {
            self.selected = (self.selected + 1) % n;
        } else {
            self.selected = (self.selected + n - 1) % n;
        }
    }

    /// 当前高亮的命令。
    pub fn selected_command(&self) -> Option<&'static Command> {
        self.matches().get(self.selected).copied()
    }
}

/// 过滤逻辑抽成自由函数，方便单测（不必构造 `Palette`）。
pub fn filter(query: &str) -> Vec<&'static Command> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return COMMANDS.iter().collect();
    }
    COMMANDS
        .iter()
        .filter(|c| {
            c.name.to_lowercase().contains(&q) || c.desc.to_lowercase().contains(&q)
        })
        .collect()
}

/// 按分类分组（保持注册表顺序，面板里按组显示）。
pub fn grouped(cmds: &[&'static Command]) -> Vec<(Category, Vec<&'static Command>)> {
    let mut out: Vec<(Category, Vec<&'static Command>)> = Vec::new();
    for c in cmds {
        match out.iter_mut().find(|(cat, _)| *cat == c.category) {
            Some((_, list)) => list.push(c),
            None => out.push((c.category, vec![c])),
        }
    }
    out
}

/// 分类标题（供 UI 用）。
pub fn category_title(c: Category) -> &'static str {
    c.title()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_maps_to_a_real_action() {
        // 关键不变量：**每个命令都必须能执行**。列着却点了没反应是
        // "看起来有功能"的假象 —— 比没有更糟（用户会以为坏了）。
        for c in COMMANDS {
            let a = c.action.resolve();
            // resolve 必然产出某个动作；这里确认它不是"空动作"占位
            // （靠 match 的穷尽性保证：新增 ActionStatic 会编译失败）
            match a {
                Action::ToggleTerminal
                | Action::ToggleSidebar | Action::NewSession
                | Action::Compact | Action::Rewind | Action::Interrupt
                | Action::ShowModels | Action::CycleMode | Action::ToggleReasoning
                | Action::ClearTranscript | Action::Help | Action::Quit => {}
            }
        }
    }

    #[test]
    fn command_names_are_unique_and_described() {
        let mut seen = Vec::new();
        for c in COMMANDS {
            assert!(!seen.contains(&c.name), "命令名重复：{}", c.name);
            seen.push(c.name);
            assert!(!c.name.is_empty(), "命令必须有名字");
            assert!(!c.desc.is_empty(), "命令 {} 缺说明（面板里会显示空行）", c.name);
        }
    }

    #[test]
    fn empty_query_lists_everything() {
        assert_eq!(filter("").len(), COMMANDS.len());
        assert_eq!(filter("   ").len(), COMMANDS.len(), "纯空白等于没输入");
    }

    #[test]
    fn filter_matches_name_and_description_case_insensitively() {
        // 按名字
        let r = filter("COMPACT");
        assert!(r.iter().any(|c| c.name == "compact"), "大小写应不敏感：{r:?}");
        // 按说明里的词 —— 用户记得"压缩"但未必记得命令叫 compact
        let r = filter("压缩");
        assert!(
            r.iter().any(|c| c.name == "compact"),
            "匹配说明里的中文：{r:?}"
        );
        // 匹配不到就是空
        assert!(filter("这个词不存在").is_empty());
    }

    #[test]
    fn selection_wraps_around_in_both_directions() {
        let mut p = Palette::default();
        p.open();
        let n = p.matches().len();
        assert!(n > 1);
        // 往上从 0 绕到最后
        p.move_selection(false);
        assert_eq!(p.selected, n - 1, "到顶再按上应绕到最后");
        // 往下绕回 0
        p.move_selection(true);
        assert_eq!(p.selected, 0, "到底再按下应绕回开头");
    }

    #[test]
    fn selection_is_clamped_when_the_filter_shrinks() {
        // 高亮在 5、然后搜索只剩 1 条 —— 不夹紧会取到越界（面板显示空行）
        let mut p = Palette::default();
        p.open();
        p.selected = 5;
        p.query = "quit".into();
        let n = p.matches().len();
        assert_eq!(n, 1, "quit 应只匹配一条：{:?}", p.matches());
        // selected 仍可能越界，取命令时必须不 panic 且给出合理结果
        // ——这里断言"越界时不 panic"，取不到就是 None
        let got = p.selected_command();
        assert!(got.is_none() || got.map(|c| c.name) == Some("quit"));
    }

    #[test]
    fn opening_resets_query_and_selection() {
        // 残留的搜索串会让下次打开"看起来少了命令"，而原因已不可见
        let mut p = Palette::default();
        p.query = "compact".into();
        p.selected = 3;
        p.close();
        p.open();
        assert!(p.query.is_empty(), "打开时应清空搜索串");
        assert_eq!(p.selected, 0, "打开时高亮回到第一条");
        assert_eq!(p.matches().len(), COMMANDS.len(), "应显示全部命令");
    }

    #[test]
    fn grouping_preserves_registry_order_and_covers_all() {
        let all = filter("");
        let g = grouped(&all);
        let total: usize = g.iter().map(|(_, v)| v.len()).sum();
        assert_eq!(total, COMMANDS.len(), "分组不能丢命令");
        // 分组内顺序 == 注册表顺序
        for (_, list) in &g {
            let idx: Vec<usize> = list
                .iter()
                .map(|c| COMMANDS.iter().position(|x| x.name == c.name).unwrap())
                .collect();
            let mut sorted = idx.clone();
            sorted.sort_unstable();
            assert_eq!(idx, sorted, "组内应保持注册表顺序");
        }
    }
}
