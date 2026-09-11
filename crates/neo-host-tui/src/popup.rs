//! 弹窗（@ 文件 / `/` 命令 / 主题 / 命令面板）
//!
//! # 为什么用"列表 + 选择"而不是"按 tab 直接插入"
//!
//! 之前 `@` 只在按 tab 时插入**最佳匹配**。问题是用户看不到有哪些候选 ——
//! 输错一个字母就被替换成了别的东西，且无从发现。
//! opencode 的做法是弹出候选列表、实时过滤、↑↓ 选择、Enter 确认。
//! 这一步的差别是"能用"与"可用"。
//!
//! # 状态与渲染分离
//!
//! 这个模块只管**选择状态**（有哪些项、选中第几项、过滤词），
//! 不碰任何 ANSI —— 渲染在 `lib.rs` 用 `Grid` 做。这样选择逻辑
//! 可以脱离终端单测。

use crate::commands;

/// 弹窗类型（决定标题与数据来源）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `@` 文件引用
    File,
    /// `/` 斜杠命令
    Slash,
    /// `ctrl+t` / `/theme` 主题选择
    Theme,
    /// `ctrl+p` 命令面板（含非斜杠动作）
    Palette,
    /// 背景选择
    Background,
    /// Logo 样式选择
    Logo,
}

impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Kind::File => "文件",
            Kind::Slash => "命令",
            Kind::Theme => "主题",
            Kind::Palette => "命令面板",
            Kind::Background => "背景",
            Kind::Logo => "Logo 样式",
        }
    }

    /// 该类型在列表中最多展示多少行（避免弹窗盖满整屏）。
    pub fn max_rows(self) -> usize {
        match self {
            Kind::File => 8,
            Kind::Slash | Kind::Palette => 8,
            Kind::Theme => 6,
            Kind::Background => 4,
            Kind::Logo => 4,
        }
    }
}

/// 选中一项后要做什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemAction {
    /// 替换输入框内容（文件引用）
    Insert(String),
    /// 执行一条已注册命令
    Run(commands::Action),
    /// 应用某个主题
    SetTheme(crate::theme::ThemeName),
    /// 应用背景纹理
    SetBackground(crate::appearance::Background),
    /// 应用 Logo 样式
    SetLogo(crate::appearance::LogoStyle),
}

/// 列表项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub label: String,
    /// 右侧说明（命令描述 / 文件来源），可为空
    pub detail: String,
    pub action: ItemAction,
    /// 所属分组（`ctrl+p` 面板按此分节；文件/主题列表留空）
    pub category: &'static str,
    /// 右侧快捷键提示（无则空串）
    pub keybinding: &'static str,
}

impl Item {
    /// 造一个不带分组/键位的项（文件与主题列表用）。
    pub fn plain(label: String, detail: String, action: ItemAction) -> Self {
        Self { label, detail, action, category: "", keybinding: "" }
    }
}

/// 弹窗状态。
#[derive(Debug, Clone)]
pub struct Popup {
    pub kind: Kind,
    /// 过滤词（`@` 后、或 `/` 后的文本）
    pub query: String,
    pub items: Vec<Item>,
    /// 当前选中下标；列表为空时无意义
    pub selected: usize,
    /// 列表是否被截断（候选多于可显示行数）
    pub truncated: bool,
}

impl Popup {
    pub fn new(kind: Kind, query: impl Into<String>) -> Self {
        Self { kind, query: query.into(), items: Vec::new(), selected: 0, truncated: false }
    }

    /// 装配候选。**选中项重置为 0**：过滤词一变，旧的选中位置就失去意义，
    /// 沿用会让光标停在毫不相干的项上。
    pub fn set_items(&mut self, items: Vec<Item>, truncated: bool) {
        self.items = items;
        self.truncated = truncated;
        self.selected = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 上下移动，**不循环**（循环会让"到底了"没有反馈）。
    pub fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        let last = self.items.len() - 1;
        let next = self.selected as isize + delta;
        self.selected = next.clamp(0, last as isize) as usize;
    }

    pub fn select(&mut self, index: usize) {
        if !self.items.is_empty() {
            self.selected = index.min(self.items.len() - 1);
        }
    }

    pub fn selected_item(&self) -> Option<&Item> {
        self.items.get(self.selected)
    }

    /// 需要滚动时，列表应从第几项开始显示（让选中项始终可见）。
    pub fn scroll_top(&self, visible_rows: usize) -> usize {
        if visible_rows == 0 || self.items.len() <= visible_rows {
            return 0;
        }
        // 选中项越过后半屏就把窗口往下推
        let half = visible_rows / 2;
        self.selected.saturating_sub(half).min(self.items.len() - visible_rows)
    }
}

// ── 候选构造 ──────────────────────────────────────────────────────────

/// 把一条命令包成 Item（`/` 列表与面板共用）。
pub fn item_of(c: &'static commands::Command) -> Item {
    Item {
        label: format!("/{}", c.name),
        detail: c.desc.to_string(),
        action: ItemAction::Run(c.action),
        category: c.category.title(),
        keybinding: c.keybinding,
    }
}

/// 斜杠命令候选（按 `query` 过滤）。
pub fn slash_items(query: &str) -> Vec<Item> {
    let mut items: Vec<(usize, usize, Item)> = commands::matches(query)
        .into_iter()
        .map(|c| {
            (
                // 排序键：(分类顺序, 是否模糊命中偏移) —— 分类内保持表内顺序
                c.category.order(),
                0usize,
                Item {
                    label: format!("/{}", c.name),
                    detail: c.desc.to_string(),
                    action: ItemAction::Run(c.action),
                    category: c.category.title(),
                    keybinding: c.keybinding,
                },
            )
        })
        .collect();
    items.sort_by_key(|(cat, off, _)| (*cat, *off));
    items.into_iter().map(|(_, _, i)| i).collect()
}

/// 命令面板候选：所有斜杠命令（面板的价值是"看得到全部"，
/// 所以不做前缀过滤之外的裁剪，只按 query 过滤）。
/// 命令面板候选。
///
/// 用**模糊匹配**（子序列）而不是子串：用户记得的是大概形状
/// （"tl" → theme/details?），精确子串会一个都不匹配。
/// 排序：先按分类，再按模糊得分（得分高的靠前）。
pub fn palette_items(query: &str) -> Vec<Item> {
    let q = query.trim();
    if q.is_empty() {
        return slash_items("");
    }
    let mut scored: Vec<(usize, i64, Item)> = Vec::new();
    for c in commands::registry() {
        let label = format!("/{}", c.name);
        // 名字与描述都参与匹配，任一命中即可
        let score = crate::input::fuzzy_score(q, &label)
            .map(|s| s as i64)
            .or_else(|| crate::input::fuzzy_score(q, c.desc).map(|s| s as i64 - 500));
        let Some(score) = score else { continue };
        scored.push((
            c.category.order(),
            -score, // 负号：分数大的排前面（sort 是升序）
            Item {
                label,
                detail: c.desc.to_string(),
                action: ItemAction::Run(c.action),
                category: c.category.title(),
                keybinding: c.keybinding,
            },
        ));
    }
    scored.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    scored.into_iter().map(|(_, _, i)| i).collect()
}

/// 主题候选。
pub fn theme_items(query: &str) -> Vec<Item> {
    let q = query.trim().to_ascii_lowercase();
    crate::theme::ThemeName::all()
        .into_iter()
        .filter(|t| q.is_empty() || t.as_str().contains(&q))
        .map(|t| {
            let d = match t {
                crate::theme::ThemeName::Neo => "NEO 默认（紫）",
                crate::theme::ThemeName::OpenCode => "opencode 官方（暖橙 / 紫）",
                crate::theme::ThemeName::Nord => "冷蓝灰",
                crate::theme::ThemeName::Gruvbox => "复古暖色",
                crate::theme::ThemeName::RosePine => "柔和玫瑰",
                crate::theme::ThemeName::Tokyonight => "霓虹夜蓝",
                crate::theme::ThemeName::Catppuccin => "柔和马卡龙",
            };
            Item::plain(t.as_str().to_string(), d.to_string(), ItemAction::SetTheme(t))
        })
        .collect()
}

/// 背景候选。
pub fn background_items(query: &str) -> Vec<Item> {
    let q = query.trim().to_ascii_lowercase();
    crate::appearance::Background::all()
        .into_iter()
        .filter(|b| q.is_empty() || b.as_str().contains(&q))
        .map(|b| {
            let d = match b {
                crate::appearance::Background::Stars => "星场（默认）：稀疏点阵，最有氛围",
                crate::appearance::Background::Dots => "点阵网格：规律安静，适合长时间看",
                crate::appearance::Background::Diagonal => "斜纹：轻微纹理感",
                crate::appearance::Background::None => "纯色：无纹理，最省心",
            };
            Item::plain(b.as_str().to_string(), d.to_string(), ItemAction::SetBackground(b))
        })
        .collect()
}

/// Logo 样式候选。
pub fn logo_items(query: &str) -> Vec<Item> {
    let q = query.trim().to_ascii_lowercase();
    crate::appearance::LogoStyle::all()
        .into_iter()
        .filter(|l| q.is_empty() || l.as_str().contains(&q))
        .map(|l| {
            let d = match l {
                crate::appearance::LogoStyle::Large => "大词标（6 行渐变，默认）",
                crate::appearance::LogoStyle::Small => "小词标（3 行）",
                crate::appearance::LogoStyle::Minimal => "极简（一行 NEO）",
                crate::appearance::LogoStyle::Hidden => "隐藏（首屏只留信息与输入框）",
            };
            Item::plain(l.as_str().to_string(), d.to_string(), ItemAction::SetLogo(l))
        })
        .collect()
}

/// 文件候选（模糊排序；`truncated` 透传自目录枚举）。
pub fn file_items(query: &str, files: &[String], limit: usize) -> (Vec<Item>, bool) {
    let q = query.trim();
    let ranked: Vec<&String> = if q.is_empty() {
        files.iter().take(limit).collect()
    } else {
        crate::input::fuzzy_rank(q, files, limit)
    };
    let items = ranked
        .into_iter()
        .map(|f| Item::plain(f.clone(), String::new(), ItemAction::Insert(format!("@{f}"))))
        .collect();
    let truncated = files.len() > limit;
    (items, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> Popup {
        let mut p = Popup::new(Kind::Slash, "");
        p.set_items(slash_items(""), false);
        p
    }

    #[test]
    fn filtering_resets_selection() {
        // 过滤词改变后，旧选中位置失去意义；不重置会让光标停在无关项上
        let mut p = p();
        p.select(3);
        assert_eq!(p.selected, 3);
        p.set_items(slash_items("th"), false);
        assert_eq!(p.selected, 0, "换候选后选中项必须回到第一项");
    }

    #[test]
    fn selection_clamps_at_both_ends() {
        let mut p = p();
        p.move_selection(-1);
        assert_eq!(p.selected, 0, "顶部不能再往上");
        for _ in 0..100 {
            p.move_selection(1);
        }
        assert_eq!(p.selected, p.items.len() - 1, "底部不能再往下（不循环）");
    }

    #[test]
    fn empty_popup_is_navigation_safe() {
        // 无候选时上下移动不得 panic（例如在空目录里按 @）
        let mut p = Popup::new(Kind::File, "");
        p.set_items(vec![], false);
        p.move_selection(1);
        p.move_selection(-1);
        assert_eq!(p.selected, 0);
        assert!(p.selected_item().is_none());
    }

    #[test]
    fn scroll_keeps_selection_visible() {
        let mut p = Popup::new(Kind::File, "");
        let items: Vec<Item> = (0..50)
            .map(|i| {
                Item::plain(
                    format!("f{i}.rs"),
                    String::new(),
                    ItemAction::Insert(format!("@f{i}.rs")),
                )
            })
            .collect();
        p.set_items(items, false);
        p.select(40);
        let top = p.scroll_top(8);
        assert!(
            top <= 40 && 40 < top + 8,
            "选中项 40 应落在窗口 [{top}, {}) 内",
            top + 8
        );
    }

    #[test]
    fn slash_items_cover_the_registry() {
        assert_eq!(slash_items("").len(), commands::registry().len());
    }

    #[test]
    fn file_items_rank_by_fuzzy_match() {
        let files = vec![
            "src/main.rs".to_string(),
            "src/parser.rs".to_string(),
            "README.md".to_string(),
        ];
        let (items, _) = file_items("main", &files, 8);
        assert_eq!(items[0].label, "src/main.rs", "最匹配的应排第一");
        assert!(matches!(&items[0].action, ItemAction::Insert(s) if s == "@src/main.rs"));
    }

    #[test]
    fn empty_query_lists_files_in_enumeration_order() {
        // 空查询不该做"模糊排序"（没有查询词就没有相关性可言）
        let files: Vec<String> = (0..20).map(|i| format!("f{i:02}.rs")).collect();
        let (items, truncated) = file_items("", &files, 8);
        assert_eq!(items.len(), 8);
        assert_eq!(items[0].label, "f00.rs");
        assert!(truncated, "候选多于上限时应标记截断");
    }

    #[test]
    fn theme_items_include_all_and_filter() {
        assert_eq!(theme_items("").len(), crate::theme::ThemeName::all().len());
        let n = theme_items("nord");
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].label, "nord");
        assert!(matches!(n[0].action, ItemAction::SetTheme(_)));
    }
}
