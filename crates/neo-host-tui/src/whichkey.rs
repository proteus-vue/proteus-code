//! Which-key —— 按键前缀提示（对齐 opencode / Emacs which-key / Vim 的 leader 提示）
//!
//! # 解决的问题
//!
//! 我们已经有二十多个键位（`ctrl+k`、`alt+b`、`]`、`v`……），但它们只静态列在
//! `/keys` 里 —— **用到的时候想不起来**。which-key 的做法是：按下前缀键后，
//! 弹出一张"接下来能按什么"的候选表，用户边按边发现。
//!
//! # 为什么不用真正的 leader 键
//!
//! opencode 用 `ctrl+x` 作 leader（Emacs 风格的两段式）。本项目已有大量
//! **单段** `ctrl+`/`alt+` 组合，再引入 leader 会让同一个键在不同上下文里
//! 含义不同，反而难记。这里把 which-key 挂在一个**显式请求**上：
//! 用户按 `ctrl+/`（"我忘了能按什么"）时弹出当前上下文的可用键。
//! 前缀触发的思路保留，但不改变既有键位语义 —— 这是取舍，写在文档里。

/// 一个上下文里可用的键位分组。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// 分组标题（如"输入 / 编辑"）
    pub title: &'static str,
    /// (按键, 说明)
    pub keys: &'static [(&'static str, &'static str)],
}

/// 当前界面上下文。
///
/// which-key 的价值在于**只显示此刻能用的键** —— 把全部键位一次倒出来
/// 等于没给提示（用户还是得自己找）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    /// 输入框有焦点（默认）
    Input,
    /// 弹窗打开（列表导航语义）
    Popup,
    /// 正在搜索转录
    Search,
    /// 转录已滚动（不在底部）
    Scrolled,
    /// 待审批
    Approval,
    /// diff 查看器
    DiffViewer,
    /// 侧栏可见
    Sidebar,
}

/// 输入框上下文。
const INPUT: &[Group] = &[
    Group {
        title: "提交 / 编辑",
        keys: &[
            ("enter", "提交任务"),
            ("alt+enter", "插入换行"),
            ("ctrl+k", "删到行尾"),
            ("ctrl+u", "删到行首"),
            ("ctrl+w", "删前一个词"),
            ("ctrl+_ / ctrl+y", "撤销 / 重做"),
            ("home / end", "行首 / 行尾"),
            ("alt+b / alt+f", "按词移动"),
            ("ctrl+g", "外部编辑器"),
        ],
    },
    Group {
        title: "历史与补全",
        keys: &[
            ("tab", "补全 @ 引用"),
            ("@ / /", "文件 / 命令列表"),
            ("↑ / ↓", "本缓冲移动，到边界走历史"),
            ("ctrl+r", "搜索历史"),
            ("ctrl+f", "搜索转录"),
        ],
    },
    Group {
        title: "界面",
        keys: &[
            ("ctrl+p", "命令面板"),
            ("ctrl+t", "切换主题"),
            ("ctrl+b", "侧栏开关"),
            ("pgup / pgdn", "滚动转录"),
            ("ctrl+e", "回到最新"),
            ("ctrl+l", "清屏"),
            ("ctrl+o", "键位提示"),
            ("ctrl+z", "挂起回 shell"),
            ("esc", "清空输入"),
            ("ctrl+c", "退出"),
        ],
    },
];

/// 按上下文挑出应当展示的分组。
pub fn groups_for(ctx: Context, sidebar_visible: bool) -> Vec<Group> {
    match ctx {
        Context::Popup => vec![Group {
            title: "列表导航",
            keys: &[
                ("↑ / ↓", "选择"),
                ("enter", "确认"),
                ("tab", "确认并继续编辑"),
                ("esc", "取消"),
                ("backspace", "缩窄过滤"),
            ],
        }],
        Context::Search => vec![Group {
            title: "搜索转录",
            keys: &[
                ("输入", "实时过滤"),
                ("enter", "确认并跳到命中"),
                ("ctrl+n / ctrl+p", "下一个 / 上一个命中"),
                ("backspace", "删一个字符"),
                ("esc", "退出搜索（保留滚动位置）"),
            ],
        }],
        Context::Approval => vec![Group {
            title: "审批",
            keys: &[
                ("y", "批准"),
                ("n", "拒绝"),
                ("d", "查看完整 diff"),
                ("ctrl+c", "退出"),
            ],
        }],
        Context::DiffViewer => vec![Group {
            title: "diff 查看器",
            keys: &[
                ("j / k", "上下移动"),
                ("] / [", "下 / 上一个 hunk"),
                ("n / p", "下 / 上一个文件"),
                ("N / P", "文件树选择"),
                ("v", "统一 / 双列"),
                ("b", "文件树开关"),
                ("g / G", "首 / 尾"),
                ("q / esc", "返回"),
            ],
        }],
        Context::Scrolled => {
            let mut v = vec![Group {
                title: "转录已上滚",
                keys: &[
                    ("pgup / pgdn", "半页滚动"),
                    ("ctrl+e", "回到最新"),
                    ("ctrl+home", "跳到最顶"),
                    ("ctrl+f", "搜索"),
                    ("ctrl+b", "侧栏开关"),
                    ("esc", "关闭弹窗 / 清空输入"),
                ],
            }];
            if sidebar_visible {
                v.push(SIDEBAR_GROUP);
            }
            v
        }
        Context::Sidebar => {
            let mut v = INPUT.to_vec();
            v.push(SIDEBAR_GROUP);
            v
        }
        Context::Input => {
            let mut v = INPUT.to_vec();
            if sidebar_visible {
                v.push(SIDEBAR_GROUP);
            }
            v
        }
    }
}

/// 侧栏分组只描述**鼠标**入口。
///
/// 键盘的 `ctrl+b` 归"界面"组管 —— 同一个键在两处各写一遍，界面上会
/// 出现两行说法不同的提示（测试就是这么抓到的）。一个键只有一个归属。
const SIDEBAR_GROUP: Group = Group {
    title: "侧栏（鼠标）",
    keys: &[
        ("点侧栏", "收起"),
        ("点右下角 ‹", "展开（收起后）"),
    ],
};

/// 从当前 UI 状态推断 which-key 上下文。
///
/// **只反映此刻真实可用的键**：例如弹窗打开时，输入框的编辑键按下去会
/// 被弹窗先吃掉，所以那些键此时"不可用"，不该出现在提示里。
pub fn detect(
    popup_open: bool,
    searching: bool,
    awaiting_approval: bool,
    diff_open: bool,
    scrolled: bool,
    sidebar_visible: bool,
) -> Context {
    if diff_open {
        return Context::DiffViewer;
    }
    if popup_open {
        return Context::Popup;
    }
    if searching {
        return Context::Search;
    }
    if awaiting_approval {
        return Context::Approval;
    }
    if scrolled {
        return Context::Scrolled;
    }
    if sidebar_visible {
        return Context::Sidebar;
    }
    Context::Input
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_context_lists_the_documented_keys() {
        let gs = groups_for(Context::Input, false);
        let all: Vec<&str> = gs.iter().flat_map(|g| g.keys.iter().map(|(k, _)| *k)).collect();
        // 这些键必须真的实现了才允许出现在提示里
        for needle in ["enter", "ctrl+k", "ctrl+u", "ctrl+_ / ctrl+y", "ctrl+g", "tab", "ctrl+p"] {
            assert!(all.contains(&needle), "输入上下文应提示 {needle}：{all:?}");
        }
    }

    #[test]
    fn popup_context_replaces_input_bindings() {
        // 弹窗打开时输入框的编辑键会被弹窗先吃掉 → 不该再提示它们。
        // 提示"按不到的键"比不提示更糟。
        let gs = groups_for(Context::Popup, false);
        let all: Vec<&str> = gs.iter().flat_map(|g| g.keys.iter().map(|(k, _)| *k)).collect();
        assert!(all.contains(&"enter"), "弹窗应提示确认");
        assert!(all.contains(&"esc"), "弹窗应提示取消");
        assert!(!all.contains(&"ctrl+z"), "弹窗下不该提示输入框的撤销");
        assert!(!all.contains(&"alt+b / alt+f"), "弹窗下不该提示按词移动");
    }

    #[test]
    fn approval_context_shows_the_three_real_options() {
        let gs = groups_for(Context::Approval, false);
        let all: Vec<&str> = gs.iter().flat_map(|g| g.keys.iter().map(|(k, _)| *k)).collect();
        assert!(all.contains(&"y") && all.contains(&"n"), "应提示批准/拒绝");
        assert!(all.contains(&"d"), "应提示可查看完整 diff");
    }

    #[test]
    fn diff_viewer_context_matches_the_real_keys() {
        let gs = groups_for(Context::DiffViewer, false);
        let all: Vec<&str> = gs.iter().flat_map(|g| g.keys.iter().map(|(k, _)| *k)).collect();
        for k in ["j / k", "] / [", "n / p", "v", "b", "q / esc"] {
            assert!(all.contains(&k), "查看器应提示 {k}：{all:?}");
        }
    }

    #[test]
    fn sidebar_group_appears_only_when_the_sidebar_is_visible() {
        let without = groups_for(Context::Input, false);
        assert!(
            !without.iter().any(|g| g.title.starts_with("侧栏")),
            "侧栏不可见时不该提示侧栏键"
        );
        let with = groups_for(Context::Input, true);
        assert!(
            with.iter().any(|g| g.title.starts_with("侧栏")),
            "侧栏可见时应提示"
        );
    }

    #[test]
    fn detect_prioritises_the_most_modal_context() {
        // diff 查看器独占输入 → 最高优先级
        assert_eq!(detect(false, false, false, true, false, false), Context::DiffViewer);
        // 弹窗盖在输入框上
        assert_eq!(detect(true, false, false, false, false, false), Context::Popup);
        // 搜索模式优先于审批提示（搜索时输入框装的是查询词）
        assert_eq!(detect(false, true, true, false, false, false), Context::Search);
        // 待审批优先于已滚动
        assert_eq!(detect(false, false, true, false, true, false), Context::Approval);
        // 已滚动优先于普通输入
        assert_eq!(detect(false, false, false, false, true, false), Context::Scrolled);
    }

    #[test]
    fn every_group_has_a_title_and_at_least_one_key() {
        // 空分组会在界面上留一块空白标题，读起来莫名其妙
        let ctxs = [
            Context::Input,
            Context::Popup,
            Context::Search,
            Context::Scrolled,
            Context::Approval,
            Context::DiffViewer,
            Context::Sidebar,
        ];
        for c in ctxs {
            for g in groups_for(c, true) {
                assert!(!g.title.is_empty(), "{c:?} 有分组缺标题");
                assert!(!g.keys.is_empty(), "{c:?} 的分组「{}」没有键", g.title);
                for (k, d) in g.keys {
                    assert!(!k.is_empty() && !d.is_empty(), "{c:?} 有空的键或说明");
                }
            }
        }
    }

    #[test]
    fn no_key_appears_twice_within_the_same_context() {
        // 同一个键在同一上下文出现两次 = 提示互相矛盾
        for c in [
            Context::Input,
            Context::Popup,
            Context::Search,
            Context::Approval,
            Context::DiffViewer,
            Context::Sidebar,
        ] {
            let mut seen: Vec<&str> = Vec::new();
            for g in groups_for(c, true) {
                for (k, _) in g.keys {
                    assert!(!seen.contains(k), "{c:?} 里键 {k} 重复出现");
                    seen.push(k);
                }
            }
        }
    }
}
