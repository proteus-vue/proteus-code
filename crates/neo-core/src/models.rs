//! 模型注册表 —— 多服务商 / 多模型，支持**运行时切换**
//!
//! # 补的是什么缺口
//!
//! 之前 `Kernel` 持有一个固定的 `Box<dyn ModelProvider>`，构造后不可更换；
//! 而协议层的 `SessionPatch.model` 字段**被接受、存进 cfg、然后从未使用** ——
//! 又一个"声明了没接线"。于是：
//!   - 用户无法在会话中换模型（只能重启进程）；
//!   - 设置页只能把"服务商"标成只读，写"运行中不可切换"。
//!
//! 这个模块把"当前用哪个模型"从**构造期决定**变成**运行期状态**。
//!
//! # 为什么注册表放在 L2 core 而不是 L3
//!
//! 它是内核**自己的内部状态**（`Kernel` 要按名字取 provider），不是某个
//! 具体后端的实现。放 L3 会让 core 依赖 L3（违反只能向下）。
//! provider 本身仍由 L3 实现并注入 —— 依赖方向不变。
//!
//! # 内存与失败语义
//!
//! 切换失败**必须报错并列出可用名字**，不能静默忽略：用户以为切了、
//! 实际还在用旧模型，是最坏的一类静默失败（会话日志会显示错误的模型）。

use std::collections::BTreeMap;

use crate::ModelProvider;

/// 手写 `Debug`：`Box<dyn ModelProvider>` 没有 `Debug`（它是个行为契据，
/// 不是数据），而 `Result<ModelRegistry, _>` 在测试里需要 `Debug`。
/// 只打印名字与当前项 —— 不试图打印 provider 内部状态（它没有可打印的契约）。
impl std::fmt::Debug for ModelRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelRegistry")
            .field("current", &self.current)
            .field("registered", &self.providers.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// 一个可选模型的元信息（供 UI 展示，不影响内核行为）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    /// 注册名（切换时用这个名字）
    pub name: String,
    /// 人类可读描述
    pub description: String,
    /// 上下文窗口（token）。0 = 未知（UI 据此不显示百分比，宁可不给也不编）。
    pub context_limit: u64,
    /// 是否可用于真实任务（mock/selftest 这类桩标 false，UI 可区分展示）
    pub production: bool,
}

/// 多 provider 注册表 + 当前选中项。
pub struct ModelRegistry {
    /// 名字 → provider。`BTreeMap` 保证遍历顺序稳定（UI 列表不会乱跳）。
    providers: BTreeMap<String, Box<dyn ModelProvider>>,
    /// 名字 → 元信息
    infos: BTreeMap<String, ModelInfo>,
    /// 当前使用的名字
    current: String,
}

impl ModelRegistry {
    /// 建注册表。至少要有 `default` 指定的那一个，否则返回错误 ——
    /// 空注册表让内核无法工作，必须在这里拦住而不是等到第一次请求。
    pub fn new(
        default: &str,
        entries: Vec<(ModelInfo, Box<dyn ModelProvider>)>,
    ) -> Result<Self, String> {
        let mut providers = BTreeMap::new();
        let mut infos = BTreeMap::new();
        for (info, p) in entries {
            // 名字与 provider 自报名不一致会让人困惑（列表显示 A、日志记 B）
            if p.name() != info.name {
                return Err(format!(
                    "注册名 {} 与 provider 自报名 {} 不一致",
                    info.name,
                    p.name()
                ));
            }
            infos.insert(info.name.clone(), info);
            providers.insert(p.name().to_string(), p);
        }
        if providers.is_empty() {
            return Err("模型注册表为空：至少要注册一个 provider".into());
        }
        if !providers.contains_key(default) {
            return Err(format!(
                "默认模型 {default} 未注册（已注册：{}）",
                providers.keys().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        Ok(Self { providers, infos, current: default.to_string() })
    }

    /// 单 provider 的便捷构造（旧的"只有一个模型"场景）。
    pub fn single(p: Box<dyn ModelProvider>) -> Self {
        let name = p.name().to_string();
        let info = ModelInfo {
            name: name.clone(),
            description: String::new(),
            context_limit: 0,
            production: true,
        };
        let mut providers = BTreeMap::new();
        providers.insert(name.clone(), p);
        let mut infos = BTreeMap::new();
        infos.insert(name.clone(), info);
        Self { providers, infos, current: name }
    }

    /// 追加或替换一个 provider（用于**运行时新增服务商**，不必重启进程）。
    ///
    /// 名字校验与 `new` 一致：注册名必须等于 provider 自报名，
    /// 否则列表显示 A、日志记 B（两处各说一套）。
    pub fn add(&mut self, info: ModelInfo, p: Box<dyn ModelProvider>) -> Result<(), String> {
        if p.name() != info.name {
            return Err(format!(
                "注册名 {} 与 provider 自报名 {} 不一致",
                info.name,
                p.name()
            ));
        }
        self.infos.insert(info.name.clone(), info);
        self.providers.insert(p.name().to_string(), p);
        Ok(())
    }

    /// 移除一个 provider。**不允许移除当前正在使用的那个** ——
    /// 那会让 `current_provider()` 立刻无 provider 可用（它不返回 Option）。
    /// 调用方应先切换到别的模型再删。
    pub fn remove(&mut self, name: &str) -> Result<bool, String> {
        if name == self.current {
            return Err(format!("{name} 正在使用中，请先切到别的模型再删除"));
        }
        self.infos.remove(name);
        Ok(self.providers.remove(name).is_some())
    }

    /// 当前模型名。
    pub fn current(&self) -> &str {
        &self.current
    }

    /// 当前 provider。
    ///
    /// 不会失败：注册表非空且 `current` 只由 `new`/`switch` 写入（两者都校验过）。
    /// 返回 `&dyn` 而非 `Option` 是刻意的 —— 调用方（内核主循环）不该被
    /// "provider 怎么可能没有"这种不可能分支干扰。
    pub fn current_provider(&self) -> &dyn ModelProvider {
        self.providers
            .get(&self.current)
            .map(|b| b.as_ref())
            .expect("current 必然存在于 providers（构造与切换均已校验）")
    }

    /// 列出全部可选项（名字有序，稳定）。
    pub fn list(&self) -> Vec<ModelInfo> {
        self.infos.values().cloned().collect()
    }

    /// 切换模型。
    ///
    /// 失败时返回**含可用名字**的错误 —— 用户要能立刻知道可以选什么。
    pub fn switch(&mut self, name: &str) -> Result<(), String> {
        if name == self.current {
            return Ok(()); // 切到同一个：幂等成功，不算错误
        }
        if !self.providers.contains_key(name) {
            return Err(format!(
                "未知模型 {name}（可选：{}）",
                self.providers.keys().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        self.current = name.to_string();
        Ok(())
    }

    /// 当前模型的上下文窗口（0 = 未知）。
    pub fn current_context_limit(&self) -> u64 {
        self.infos.get(&self.current).map(|i| i.context_limit).unwrap_or(0)
    }

    /// 已注册的模型数。
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ModelDelta, ModelStream};

    /// 测试用 provider：自报名字，回一句话。
    struct Named(&'static str);
    impl ModelProvider for Named {
        fn name(&self) -> &str { self.0 }
        fn stream(&self, _r: &crate::ModelRequest<'_>) -> ModelStream {
            Box::new(vec![ModelDelta::Text(format!("from {}", self.0))].into_iter())
        }
    }

    fn info(name: &str) -> ModelInfo {
        ModelInfo {
            name: name.to_string(),
            description: format!("{name} 的描述"),
            context_limit: 64_000,
            production: true,
        }
    }

    fn reg() -> ModelRegistry {
        ModelRegistry::new(
            "a",
            vec![
                (info("a"), Box::new(Named("a"))),
                (info("b"), Box::new(Named("b"))),
            ],
        )
        .unwrap()
    }

    #[test]
    fn starts_on_the_default_model() {
        let r = reg();
        assert_eq!(r.current(), "a");
        assert_eq!(r.current_provider().name(), "a");
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn switching_changes_the_provider_used() {
        let mut r = reg();
        r.switch("b").unwrap();
        assert_eq!(r.current(), "b");
        assert_eq!(r.current_provider().name(), "b", "切换后必须换 provider");
    }

    #[test]
    fn switching_to_an_unknown_model_lists_the_available_ones() {
        // 关键：错误信息要让人立刻知道能选什么，而不是只说"失败了"
        let mut r = reg();
        let e = r.switch("不存在的").unwrap_err();
        assert!(e.contains("不存在的"), "应指出请求的名字：{e}");
        assert!(e.contains('a') && e.contains('b'), "应列出可用名字：{e}");
        assert_eq!(r.current(), "a", "失败不该改变当前模型");
    }

    #[test]
    fn switching_to_the_same_model_is_idempotent_not_an_error() {
        // 用户重复选择同一项不该报错（UI 上很容易触发）
        let mut r = reg();
        assert!(r.switch("a").is_ok());
        assert_eq!(r.current(), "a");
    }

    #[test]
    fn mismatched_name_is_rejected_at_construction() {
        // 注册名 A、provider 自报 B → 列表显示 A 而日志记 B，必须拦住
        let e = ModelRegistry::new("a", vec![(info("a"), Box::new(Named("b")))]).unwrap_err();
        assert!(e.contains("不一致"), "{e}");
    }

    #[test]
    fn empty_registry_is_rejected() {
        let e = ModelRegistry::new("a", vec![]).unwrap_err();
        assert!(e.contains("为空"), "{e}");
    }

    #[test]
    fn default_must_exist() {
        let e = ModelRegistry::new("缺失", vec![(info("a"), Box::new(Named("a")))]).unwrap_err();
        assert!(e.contains("未注册"), "{e}");
        assert!(e.contains('a'), "应列出已注册的名字：{e}");
    }

    #[test]
    fn list_is_sorted_and_stable() {
        // 列表顺序稳定，UI 上不会每次打开都换位置
        let r = reg();
        let names: Vec<String> = r.list().into_iter().map(|i| i.name).collect();
        assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn single_provider_convenience_works() {
        let r = ModelRegistry::single(Box::new(Named("only")));
        assert_eq!(r.current(), "only");
        assert_eq!(r.len(), 1);
        assert_eq!(r.current_context_limit(), 0, "单 provider 便捷构造不编造上下文窗口");
    }

    #[test]
    fn context_limit_follows_the_current_model() {
        let mut r = ModelRegistry::new(
            "small",
            vec![
                (
                    ModelInfo { name: "small".into(), description: String::new(), context_limit: 32_000, production: true },
                    Box::new(Named("small")),
                ),
                (
                    ModelInfo { name: "big".into(), description: String::new(), context_limit: 128_000, production: true },
                    Box::new(Named("big")),
                ),
            ],
        )
        .unwrap();
        assert_eq!(r.current_context_limit(), 32_000);
        r.switch("big").unwrap();
        assert_eq!(r.current_context_limit(), 128_000, "切换后上下文窗口应跟着变");
    }
}
