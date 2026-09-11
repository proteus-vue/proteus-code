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
}

/// 全部可用命令。顺序即 `/` 列表的展示顺序（高频在前）。
pub fn registry() -> &'static [Command] {
    &[
        Command { name: "help", aliases: &[], desc: "显示帮助", action: Action::Help },
        Command { name: "keys", aliases: &["keybinds"], desc: "键盘快捷键", action: Action::Keys },
        Command { name: "theme", aliases: &["themes"], desc: "切换配色主题", action: Action::ThemePicker },
        Command { name: "new", aliases: &["clear"], desc: "开始新对话（清空当前转录）", action: Action::NewSession },
        Command { name: "compact", aliases: &["summarize"], desc: "压缩上下文以腾出预算", action: Action::Compact },
        Command { name: "exit", aliases: &["quit", "q"], desc: "退出 Neo", action: Action::Quit },
    ]
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
  / 唤起命令列表；常用命令见下表（也可 ctrl+p 打开命令面板）

按键
  tab          在 @ 上下文中补全
  ↑ / ↓        历史导航；弹窗打开时用于选择
  ctrl+r       搜索历史
  ctrl+g       用 $EDITOR 编辑当前输入
  ctrl+p       命令面板
  ctrl+t       切换到下一个主题
  ctrl+l       清屏
  esc          关闭弹窗
  ctrl+c       退出"
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
