//! HTTP 端点的访问令牌。
//!
//! # 为什么必须有（不是一个"锦上添花"的加固）
//!
//! 只绑 `127.0.0.1` 限制的是**网络来源**，不是**谁能访问**：
//!
//! 1. **本机任意进程**都能枚举回环端口并直接调用端点 —— `POST /api/turn`
//!    交给内核写工作区、`GET /api/approve` 可批准挂起的审批、`POST /api/goal`
//!    可下发编排。端口随机只降低可见性，不是访问控制。
//! 2. 更严重的是**浏览器里的任意网页**：本服务的端点全是"简单请求"
//!    （`POST` + 纯文本正文，不触发 CORS 预检），所以恶意页面可以
//!    `fetch('http://127.0.0.1:<port>/api/turn', {method:'POST', body:'...'})`
//!    —— 它读不到响应，但**请求已经送达并生效**。这是 CSRF，也是最现实
//!    的攻击面：用户打开一个网页就够了。
//!
//! 令牌把这两条同时堵死：进程猜不到 32 字节随机值，网页也拿不到
//! （令牌在**另一个源**的 URL 里，`fetch` 无法读取跨源 URL）。
//!
//! # 传递方式：query 参数
//!
//! 必须同时适配 `fetch` 与 `EventSource`，而 `EventSource` **不能设自定义
//! 请求头** —— 所以 query 参数（`?token=`）是浏览器侧唯一的通用方式。
//! 程序化客户端另有 `X-Neo-Token` 头可选（同样的值，不必塞进 URL）。
//!
//! 令牌会出现在浏览器地址栏与 `Referer` 里。这在本地回环场景可接受：
//! 内置页面**零外部资源**（无外链、无 CDN），不存在把 `Referer` 发到
//! 第三方的路径。
//!
//! # 诚实边界
//!
//! 这是**单进程生命周期内的**一次性令牌，不是多用户鉴权：进程重启即换新值，
//! 也没有"用户/角色"概念。相应地，它防的是"本机其它程序"与"浏览器里的
//! 恶意页面"，**不防**能读本进程内存或 `/dev/urandom` 的对手。

/// 令牌字节数（256 位）。
const TOKEN_BYTES: usize = 32;

/// 生成一个新令牌（十六进制字符串）。
pub fn generate_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    if !os_random(&mut bytes) {
        weak_token_bytes(&mut bytes);
    }
    to_hex(&bytes)
}

/// 平台随机源。成功返回 true。
#[cfg(unix)]
fn os_random(out: &mut [u8]) -> bool {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(out))
        .is_ok()
}

/// 非 unix 平台：本项目尚未提供系统随机源。见 `weak_token_bytes` 的说明。
#[cfg(not(unix))]
fn os_random(_out: &mut [u8]) -> bool {
    false
}

/// 降级随机源（`/dev/urandom` 不可用时）。
///
/// **它不是密码学安全的**：种子取自时间、pid 与一个栈地址，同机对手可以把
/// 熵估到很小。之所以仍作降级而非直接 panic：这条路只在非 unix 或
/// `/dev/urandom` 打不开时触发，而 Windows/Linux 的沙箱本就未实现
/// （受限档位 fail-closed），桌面宿主实际只在 macOS 上跑 —— 那里走
/// `/dev/urandom`。诚实标注好过假装安全，也好过让宿主直接起不来。
fn weak_token_bytes(out: &mut [u8]) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let stack_addr = &nanos as *const u64 as u64;
    let mut x = nanos ^ (std::process::id() as u64).rotate_left(17) ^ stack_addr.rotate_left(31);
    for chunk in out.chunks_mut(8) {
        // splitmix64：只是个扩散函数，不提供额外熵
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        for (dst, src) in chunk.iter_mut().zip(z.to_le_bytes().iter()) {
            *dst = *src;
        }
    }
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap_or('0'));
    }
    s
}

/// 恒定时间比较（避免按字节短路泄漏"猜对了几位"）。
///
/// 长度不等会直接返回 false —— 令牌长度不是秘密（固定 64 字符十六进制），
/// 这条短路不泄漏有用信息。
pub fn token_matches(expected: &str, got: &str) -> bool {
    let a = expected.as_bytes();
    let b = got.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// 从请求里取令牌：先看 `?token=`，再看 `X-Neo-Token` 头。
///
/// 两者都不做 URL 解码之外的处理 —— 令牌是十六进制，不需要百分号解码，
/// 但客户端可能整体编码过 query（`encodeURIComponent` 不会改十六进制）。
pub fn extract_token<'a>(query: Option<&'a str>, header: Option<&'a str>) -> Option<&'a str> {
    if let Some(q) = query {
        for pair in q.split('&') {
            if let Some(v) = pair.strip_prefix("token=") {
                if !v.is_empty() {
                    return Some(v);
                }
            }
        }
    }
    header.filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_64_hex_chars_and_unique() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), TOKEN_BYTES * 2);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()), "必须是十六进制：{a}");
        assert_ne!(a, b, "两次生成不能相同");
    }

    #[test]
    fn comparison_accepts_only_the_exact_token() {
        let t = generate_token();
        assert!(token_matches(&t, &t));
        assert!(!token_matches(&t, ""));
        assert!(!token_matches(&t, "short"));
        assert!(!token_matches(&t, &format!("{t}0")));
        // 只差最后一位也必须判否（不能被前缀匹配骗过）
        let mut last_diff = t.clone();
        last_diff.pop();
        last_diff.push(if t.ends_with('0') { '1' } else { '0' });
        assert!(!token_matches(&t, &last_diff));
    }

    #[test]
    fn extraction_prefers_the_query_and_falls_back_to_the_header() {
        assert_eq!(extract_token(Some("token=abc"), None), Some("abc"));
        assert_eq!(extract_token(Some("a=1&token=abc&b=2"), None), Some("abc"));
        assert_eq!(extract_token(None, Some("xyz")), Some("xyz"));
        // 空的 query 值不算数，应继续看头
        assert_eq!(extract_token(Some("token="), Some("xyz")), Some("xyz"));
        assert_eq!(extract_token(Some("other=1"), None), None);
        assert_eq!(extract_token(None, None), None);
    }
}
