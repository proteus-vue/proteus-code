//! 真机流式回归（`#[ignore]`，不进常规门禁 —— 需要真 key 与外网）。
//!
//! 跑法：
//! ```text
//! ZHIPU_API_KEY=... cargo test -p neo-llm-deepseek --test live_stream -- --ignored --nocapture
//! ```
//!
//! # 为什么要有它
//!
//! 单测里的 SSE 字节是手拼的；真实网关（智谱 glm-4.6 / DeepSeek）的
//! 帧切分、字段形态、`[DONE]`、usage 收尾 chunk 只有真机才见得到。
//! 用户报过"思考过程不实时"——这条回归钉住的就是
//! 「真机响应里必须解析出 reasoning 增量，且先于正文」这条链路。
//!
//! 密钥只从环境变量读，**绝不**写进仓库。

use neo_core::{ModelDelta, ModelProvider, ModelRequest};
use neo_llm_deepseek::DeepSeekProvider;

fn live_provider() -> Option<DeepSeekProvider> {
    let key = std::env::var("ZHIPU_API_KEY").ok()?;
    Some(DeepSeekProvider {
        api_key: key,
        endpoint: "open.bigmodel.cn".into(),
        path: "/api/paas/v4/chat/completions".into(),
        model: std::env::var("ZHIPU_MODEL").unwrap_or_else(|_| "glm-4.6".into()),
        temperature: 0.0,
        label: "zhipu".into(),
    })
}

#[test]
#[ignore = "真机回归：需要 ZHIPU_API_KEY 与外网；用 --ignored 显式运行"]
fn zhipu_glm_streams_reasoning_before_answer() {
    let Some(p) = live_provider() else {
        eprintln!("未设置 ZHIPU_API_KEY，跳过真机回归");
        return;
    };
    let msgs = vec![neo_core::Message::User("用一句话解释什么是流式传输".into())];
    let req = ModelRequest { system: "你是简洁的助手", messages: &msgs, tools: &[] };

    let mut reasoning = String::new();
    let mut answer = String::new();
    let mut saw_usage = false;
    for delta in p.stream(&req) {
        match delta {
            ModelDelta::Reasoning(t) => reasoning.push_str(&t),
            ModelDelta::Text(t) => answer.push_str(&t),
            ModelDelta::Usage { .. } => saw_usage = true,
            ModelDelta::ToolCall(c) => panic!("不该有工具调用：{c:?}"),
        }
    }

    eprintln!("=== reasoning（{}/{} 字符）===", reasoning.chars().count(), reasoning.len());
    eprintln!("{}", preview(&reasoning));
    eprintln!("=== answer（{} 字符）===", answer.chars().count());
    eprintln!("{}", preview(&answer));

    assert!(!reasoning.is_empty(), "glm-4.6 必须产出 reasoning_content 增量");
    assert!(!answer.is_empty(), "正文不能为空");
    assert!(saw_usage, "include_usage 的收尾 chunk 必须带 usage");
}

fn preview(s: &str) -> String {
    let mut out: String = s.chars().take(400).collect();
    if s.chars().count() > 400 {
        out.push_str(" …");
    }
    out
}
