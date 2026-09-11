//! 技能（Skill）注册表：`$name` 引用解析出的可注入内容。
//!
//! # 为什么是"注册表 + 加载器"两个概念
//!
//! - [`Skill`] 是**数据**（L2 定义）：名字 + 正文。内核只认这个结构，
//!   不知道技能从磁盘哪个目录来。
//! - 加载器（[`SkillLoader`] 的实现在 L3）负责**发现**：扫目录、找 SKILL.md、
//!   解析 frontmatter。内核通过注入拿到 `Vec<Skill>`，不依赖文件系统布局。
//!
//! 这样换一种技能来源（配置文件、远程、内置常量）不需要动内核 ——
//! 与 ModelProvider / SandboxBackend 同一个 seam 思路。
//!
//! # 诚实边界
//!
//! 名字冲突时**后者覆盖前者**并保留到注册表里的是**最后加载的那份**；
//! 不静默去重成"第一个"。调用方（L3 加载器）负责决定加载顺序。

/// 一个技能：名字 + 正文（注入模型上下文的完整内容）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    /// 技能描述（来自 frontmatter 的 `description`，没有则为空）。
    pub description: String,
    /// 技能正文（frontmatter 之后的全部内容）。注入时**只注入这一段**，
    /// 不注入 frontmatter —— 那是元数据，不是给模型的指令。
    pub body: String,
}

impl Skill {
    pub fn new(name: impl Into<String>, description: impl Into<String>, body: impl Into<String>) -> Self {
        Self { name: name.into(), description: description.into(), body: body.into() }
    }

    /// 从 SKILL.md 原文解析。
    ///
    /// 格式（与 docs/ 下的技能文件一致）：
    /// ```text
    /// ---
    /// name: foo
    /// description: 一段话，可能很长
    /// ---
    /// 正文……
    /// ```
    ///
    /// frontmatter 缺失时：名字退回 `fallback_name`，整篇当正文。
    /// **不报错** —— 一个没有 frontmatter 的 Markdown 仍可能是有用的技能正文，
    /// 拒绝加载比宽松加载更糟（用户看不到任何东西，也不知道为什么）。
    pub fn parse(raw: &str, fallback_name: &str) -> Skill {
        let normalized = raw.strip_prefix('\u{feff}').unwrap_or(raw);
        let mut fm_name = String::new();
        let mut fm_desc = String::new();
        let mut body_start = 0usize;

        let mut lines = normalized.split_inclusive('\n');
        if let Some(first) = lines.next() {
            if first.trim_end() == "---" {
                let mut offset = first.len();
                let mut closed = false;
                for line in lines.by_ref() {
                    let t = line.trim_end_matches(['\n', '\r']).trim();
                    offset += line.len();
                    if t == "---" {
                        closed = true;
                        break;
                    }
                    if let Some(v) = t.strip_prefix("name:") {
                        if fm_name.is_empty() {
                            fm_name = v.trim().trim_matches('"').to_string();
                        }
                    } else if let Some(v) = t.strip_prefix("description:") {
                        if fm_desc.is_empty() {
                            fm_desc = v.trim().trim_matches('"').to_string();
                        }
                    }
                }
                // 只有**闭合**的 frontmatter 才被采信：未闭合的文件是残缺的，
                // 采信其中解析到一半的 name 会让技能以错误的名字注册
                // （用户 `$真名` 找不到、`$半个名字` 反而命中）。
                if closed {
                    body_start = offset;
                } else {
                    fm_name.clear();
                    fm_desc.clear();
                }
            }
        }

        let body = normalized[body_start.min(normalized.len())..].trim_start_matches('\n').to_string();
        let name = if fm_name.is_empty() { fallback_name.to_string() } else { fm_name };
        Skill { name, description: fm_desc, body }
    }
}

/// 技能注册表：名字 → 技能。查找不做大小写/前缀模糊匹配，
/// 因为 `$Name` 与 `$name` 引用的应当是同一个技能还是不同技能，
/// 只有用户知道 —— 猜错会注错内容，比"找不到"更糟。
#[derive(Debug, Default, Clone)]
pub struct SkillRegistry {
    skills: Vec<Skill>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self { skills: Vec::new() }
    }

    pub fn from_vec(skills: Vec<Skill>) -> Self {
        Self { skills }
    }

    pub fn add(&mut self, s: Skill) {
        self.skills.push(s);
    }

    /// 精确名字查找。
    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.name == name)
    }

    pub fn names(&self) -> Vec<&str> {
        self.skills.iter().map(|s| s.name.as_str()).collect()
    }

    pub fn len(&self) -> usize {
        self.skills.len()
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_body() {
        let raw = "---\nname: demo\ndescription: 一段描述\n---\n\n# 正文\n内容\n";
        let s = Skill::parse(raw, "fallback");
        assert_eq!(s.name, "demo");
        assert_eq!(s.description, "一段描述");
        assert_eq!(s.body, "# 正文\n内容\n");
        assert!(!s.body.contains("description"), "frontmatter 不得进入正文");
    }

    #[test]
    fn no_frontmatter_falls_back_to_name_and_keeps_whole_body() {
        // 没有 frontmatter 时**不报错**：整篇当正文，名字用兜底值。
        let s = Skill::parse("# 就是一篇普通文档\n", "file-stem");
        assert_eq!(s.name, "file-stem");
        assert!(s.description.is_empty());
        assert_eq!(s.body, "# 就是一篇普通文档\n");
    }

    #[test]
    fn unterminated_frontmatter_is_not_consumed() {
        // 只有起始 `---` 没有结束：不能把整篇都当 frontmatter 吃掉，
        // 否则正文为空 —— 技能"加载成功"但什么也没注入，是最隐蔽的失败。
        let raw = "---\nname: x\n正文没有结束标记\n";
        let s = Skill::parse(raw, "fb");
        assert_eq!(s.name, "fb");
        assert!(s.body.contains("正文没有结束标记"));
    }

    #[test]
    fn registry_exact_lookup_only() {
        let mut r = SkillRegistry::new();
        r.add(Skill::new("Neo", "", "A"));
        r.add(Skill::new("neo", "", "B"));
        assert_eq!(r.get("Neo").unwrap().body, "A");
        assert_eq!(r.get("neo").unwrap().body, "B");
        assert!(r.get("NEO").is_none(), "不做大小写模糊匹配");
        assert_eq!(r.len(), 2);
    }
}
