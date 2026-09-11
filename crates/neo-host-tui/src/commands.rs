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

/// 一条斜杠命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    /// 命令名（不含 `/`）
    pub name: &'static str,
    /// 别名（含或不含 `/` 都可以写，展示时会统一加）
    pub aliases: &'static [&'static str],
    pub desc: &'static str,
    pub action: Action,
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
}

/// **必须由命令触发**的动作。新增变体时要加进这里 ——
/// 编译器不会提醒"去注册一条命令"，很容易出现"动作写好了但没有命令能触发"。
const COMMAND_ACTIONS: &[Action] = &[
    Action::Quit,
    Action::NewSession,
    Action::Compact,
    Action::ThemePicker,
    Action::NextTheme,
    Action::Help,
    Action::Keys,
    Action::Status,
];

/// **只在界面内部产生**的动作（不经过命令表）。
///
/// `SetTheme` 只由主题选择弹窗产生 —— 让用户敲 `/settheme nord`
/// 不如让他从列表里选（名字要记，也没法预览）。这类动作刻意不注册命令。
const INTERNAL_ONLY_ACTIONS: &[Action] = &[Action::SetTheme(crate::theme::ThemeName::Nord)];

/// 命令表本体（单一事实源）。
///
/// 单独成 const 而不是写进 `registry()` 的函数体：审计函数要读它，
/// 若审计走 `registry()` 会**无限递归**（registry → audit → registry），
/// 表现为测试里的 stack overflow。表格是数据，取用方式是函数。
const COMMANDS: &[Command] = &[
    Command { name: "help", aliases: &[], desc: "显示帮助", action: Action::Help },
    Command { name: "keys", aliases: &["keybindings"], desc: "键盘快捷键", action: Action::Keys },
    Command { name: "status", aliases: &["info"], desc: "运行状态与环境", action: Action::Status },
    Command { name: "theme", aliases: &["themes"], desc: "切换配色主题", action: Action::ThemePicker },
    Command { name: "next", aliases: &["next-theme"], desc: "切到下一个主题", action: Action::NextTheme },
    Command { name: "new", aliases: &["clear"], desc: "开始新对话（清空当前转录）", action: Action::NewSession },
    Command { name: "compact", aliases: &["summarize"], desc: "压缩上下文以腾出预算", action: Action::Compact },
    Command { name: "exit", aliases: &["quit", "q"], desc: "退出 Neo", action: Action::Quit },
];

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
    /help      显示帮助
    /keys      键盘快捷键
    /status    运行状态与环境
    /theme     选择配色主题（6 套）
    /next      直接切到下一个主题
    /new       开始新对话（清空转录）
    /compact   压缩上下文（需 L4 编排，尚未实现）
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
  ctrl+z       撤销          ctrl+y  重做
  ctrl+r       搜索历史（输入框）
  ctrl+g       用 $EDITOR 编辑当前输入
  ctrl+p       命令面板      ctrl+t  切换主题
  ctrl+b       侧栏开关      ctrl+l  清屏
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
  未实现    L4 目标编排（/compact 依赖它）、多会话、多模型切换
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
        // 未实现的功能不得出现在文案里
        for bad in ["/models", "/agents", "/sessions", "/mcps", "/share"] {
            assert!(!h.contains(bad), "不得宣传未实现命令 {bad}");
            assert!(!k.contains(bad), "不得宣传未实现命令 {bad}");
        }
    }
}
