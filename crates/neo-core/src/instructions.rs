//! 项目指令（`AGENTS.md`）的**数据表示** —— L2 定义，L3 负责从磁盘发现。
//!
//! # 为什么数据在 L2、发现（扫目录/级联）在 L3
//!
//! 与 `skills` 同一个理由：内核只需要"有一段必须遵守的项目约定文本"，
//! 不需要知道它来自 `~/.neo/AGENTS.override.md` 还是 `<repo>/crates/x/AGENTS.md`。
//! 换一种指令来源（远程、配置内联）不需要动内核。

/// 已合并的项目指令。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Instructions {
    /// 参与合并的文件（展示用，按加载顺序）。
    pub sources: Vec<String>,
    /// 注入系统提示词的文本（空 = 没有任何指令）。
    pub block: String,
    /// 是否因超过字节上限被截断（**必须如实上报**给模型与用户）。
    pub truncated: bool,
}

impl Instructions {
    pub fn is_empty(&self) -> bool {
        self.block.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.block.len()
    }

    /// 一行摘要（事件与宿主展示用）。
    pub fn summary(&self) -> String {
        if self.sources.is_empty() {
            return "未找到项目指令（AGENTS.md）".to_string();
        }
        format!(
            "已加载项目指令：{}（{} 字节{}）",
            self.sources.join(" + "),
            self.bytes(),
            if self.truncated { "，已截断" } else { "" }
        )
    }
}
