//! 验证手写 HTTP 的**帧构造**是否正确 —— 对公共回显端点真发一次。
//!
//! 为什么值得单独测：手写 HTTP 最容易错的不是解析，而是**帧**：
//! `Content-Length` 算错、缺 `Host`、忘了 `Connection: close`，都会让服务器
//! 返回 400 或直接挂住。这类错误只有真发请求才暴露。
//!
//! 不依赖任何 API key：打 httpbin 的 `/post`，它回显收到的 body 与头。
//! 网络不可达时**跳过并打印原因**，不让离线环境误报失败。

use dsh_llm_deepseek::probe_post;

#[test]
fn http_post_frame_is_accepted_by_a_real_server() {
    match probe_post("httpbin.org", "/post", r#"{"neo":"probe"}"#) {
        Ok((status, body)) => {
            assert_eq!(status, 200, "帧构造应被接受，实际 status={status}，body={body}");
            // 回显含我们发的内容 → 证明 body 完整送达（Content-Length 正确）
            assert!(body.contains("neo") && body.contains("probe"), "回显应含请求体，实际：{body}");
        }
        Err(e) if e.contains("无法启动 openssl") => eprintln!("[SKIP] 无 openssl：{e}"),
        Err(e) if e.contains("未收到") => eprintln!("[SKIP] 网络不可达：{e}"),
        Err(e) => panic!("HTTP 帧构造有问题：{e}"),
    }
}
