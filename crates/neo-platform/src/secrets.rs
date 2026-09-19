//! 敏感信息检测：**在内容被聚合展示之前**认出疑似凭据。
//!
//! # 它为什么存在
//!
//! 但凡把仓库里的文档**聚合成一处**（Repo Wiki、导出、给模型的上下文包），
//! 就存在一条真实的泄露路径：某个 `.md` 里贴过一段 `.env`、一个 curl 示例里
//! 带了真 key、`docs/` 里躺着一份私钥 —— 内容原本分散着没人注意，聚合之后
//! 被**打包**成一个入口，一次全露。
//!
//! 所以这里把「绝不打包开发者密钥等私密信息」从**声明**变成**可判定的检测**：
//! 聚合方在把内容收进去之前先过一遍本模块，命中即**排除并如实上报**。
//!
//! # 三条设计纪律（都不是风格问题）
//!
//! 1. **检测结果里不得出现密钥原文**。命中描述只给"哪条规则、哪个变量名、
//!    多长"，**不回显字符**。否则检测器自己就成了一条泄露通道 ——
//!    它会把密钥抄进界面、抄进日志、抄进错误信息。这是最容易写错的一处：
//!    "把命中的内容显示出来方便排查"是自然冲动，也正是必须克制的冲动。
//! 2. **宁可多报，不可漏报**（在已成形的 key 格式上）。漏报的代价是密钥
//!    被聚合进产物；误报的代价是某个文档没被收进 Wiki、并**明确告知原因**。
//!    两者不对称，故规则偏保守。但见第 3 条 —— 不能保守到天天误报的地步。
//! 3. **占位符必须放过**。文档里大量出现 `sk-YOUR_KEY_HERE`、`${API_KEY}`、
//!    `AKIAIOSFODNN7EXAMPLE`（AWS 官方示例）这类**教学用占位**。把它们一律
//!    拦下的话，Wiki 会被自己的守卫掏空，而"天天误报的守卫"下一步就是被关掉。
//!    所以每条规则都要过一道 [`looks_like_placeholder`]。
//!
//! # 不用正则
//!
//! 本仓没有 `regex` 依赖，也不为一个检测器引入（依赖面是刻意收窄的）。
//! 下面全是手写的字符扫描 —— 规则少且形状固定，足够。

/// 命中类型。**故意做得粗**：它决定给用户看的那句话，不做精确分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    /// PEM / PuTTY 私钥块（`-----BEGIN ... PRIVATE KEY-----`）
    PrivateKeyBlock,
    /// 已知服务商的密钥前缀（`sk-` / `ghp_` / `AKIA` / `AIza` …）
    ProviderKey,
    /// 形如 `NAME=value` 且 NAME 是敏感名（`*_API_KEY` / `*_TOKEN` / …）
    EnvAssignment,
    /// URL 内嵌凭据（`scheme://user:pass@host`）
    UrlCredentials,
}

impl SecretKind {
    /// 给用户看的一句话（界面直接显示，不要自己编）。
    pub fn explain(self) -> &'static str {
        match self {
            Self::PrivateKeyBlock => "私钥块",
            Self::ProviderKey => "服务商密钥",
            Self::EnvAssignment => "密钥赋值",
            Self::UrlCredentials => "URL 内嵌凭据",
        }
    }
}

/// 一处命中。
///
/// ⚠️ **不含密钥原文**（见模块纪律第 1 条）。`detail` 只描述"在哪、多长、什么名字"。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretHit {
    pub kind: SecretKind,
    /// 行号（1 起）。用于告诉开发者"去改哪一行"。
    pub line: usize,
    /// 人类可读的定位信息（**不含密钥字符**）。
    pub detail: String,
}

/// 扫描一段文本，返回所有命中（按行号升序）。
///
/// 逐行扫描：所有规则的形状都是"行内"的，且行号正是开发者要的信息。
/// 跨行密钥（PEM 块）由 `BEGIN` 那一行代表 —— 报出首行足够定位。
pub fn find_secrets(text: &str) -> Vec<SecretHit> {
    let mut hits = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line_no = i + 1;
        if let Some(detail) = private_key_block(line) {
            hits.push(SecretHit { kind: SecretKind::PrivateKeyBlock, line: line_no, detail });
            // 私钥块：本行已定性，不再往下判（其余规则对它没有增量信息）
            continue;
        }
        if let Some(detail) = url_credentials(line) {
            hits.push(SecretHit { kind: SecretKind::UrlCredentials, line: line_no, detail });
        }
        if let Some(detail) = env_assignment(line) {
            hits.push(SecretHit { kind: SecretKind::EnvAssignment, line: line_no, detail });
        }
        for detail in provider_keys(line) {
            hits.push(SecretHit { kind: SecretKind::ProviderKey, line: line_no, detail });
        }
    }
    hits
}

/// 这段文本里有没有疑似凭据（聚合方只关心"能不能收"）。
pub fn contains_secret(text: &str) -> bool {
    !find_secrets(text).is_empty()
}

// ── 规则 1：私钥块 ────────────────────────────────────────────────────────

/// `-----BEGIN <something> PRIVATE KEY-----`（含 PuTTY 的私钥头）。
///
/// 判定只看**头部**：抓到一个 `BEGIN ... PRIVATE KEY` 就足以定性，不必看正文
/// （正文是 base64，逐行判反而容易把它当成"高熵字符串"从而报一堆重复命中）。
fn private_key_block(line: &str) -> Option<String> {
    // PuTTY 格式：`PuTTY-User-Key-File-2: ssh-rsa`（无 PEM 头）
    if line.contains("PuTTY-User-Key-File") {
        return Some("PuTTY 私钥文件头".into());
    }
    let start = line.find("-----BEGIN ")?;
    let rest = &line[start + "-----BEGIN ".len()..];
    let end = rest.find("-----")?;
    let label = rest[..end].trim();
    // 大小写不敏感：PEM 按惯例大写，但手写/生成的变体不该漏
    if label.to_ascii_uppercase().contains("PRIVATE KEY") {
        return Some(format!("PEM 私钥块 `BEGIN {}`", label));
    }
    None
}

// ── 规则 2：已知服务商前缀 ────────────────────────────────────────────────

/// (前缀, 尾部长度的下限)。
///
/// 尾部长度取各自格式的**实际长度下限**再留一点余量：太短会让规则变成
/// "凡是 sk- 开头都拦"（文档里 `sk-` 常作为通配提到），太长会漏掉旧格式。
const PROVIDER_PREFIXES: &[(&str, usize)] = &[
    ("sk-proj-", 40), // OpenAI 项目级 key（比通用 sk- 更长，故排在前面）
    ("sk-ant-", 40),  // Anthropic
    ("sk-", 24),      // OpenAI / DeepSeek 等通用
    ("sk_live_", 20), // Stripe
    ("sk_test_", 20),
    ("ghp_", 30), // GitHub PAT / OAuth / user / server / refresh
    ("gho_", 30),
    ("ghu_", 30),
    ("ghs_", 30),
    ("ghr_", 30),
    ("github_pat_", 20),
    ("glpat-", 18), // GitLab
    ("xoxb-", 20),  // Slack
    ("xoxp-", 20),
    ("xoxa-", 20),
    ("xoxr-", 20),
    ("npm_", 30), // npm
    ("pypi-", 20), // PyPI
    ("hf_", 20),  // HuggingFace
    ("AIza", 30), // Google API key
    ("dop_v1_", 40), // DigitalOcean
];

/// 扫一行里所有命中 `PROVIDER_PREFIXES` 的候选。
///
/// 返回 `(起止字节, 描述)`，由调用方按**位置去重**（见 [`provider_keys`]）。
pub(crate) fn provider_spans(line: &str) -> Vec<(std::ops::Range<usize>, String)> {
    let mut out: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    // AWS 的 `AKIA`/`ASIA` 只有 4 个字符，在任何文本里都可能偶然出现，
    // 所以它单独走一条更严的规则（全大写字母数字、精确长度附近）。
    aws_access_key(line, &mut out);

    for (prefix, min_tail) in PROVIDER_PREFIXES {
        let mut from = 0;
        while let Some(pos) = line[from..].find(prefix) {
            let start = from + pos;
            let tail = &line[start + prefix.len()..];
            let token_tail: String = tail
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if token_tail.len() >= *min_tail {
                let full: String = prefix.chars().chain(token_tail.chars()).collect();
                let end = start + full.len().min(line.len() - start);
                if !looks_like_placeholder(&full) {
                    // ⚠️ 只报前缀与长度，**不回显密钥**
                    out.push((
                        start..end,
                        format!("`{}…` 形状的凭据（共 {} 字符）", prefix, full.len()),
                    ));
                }
            }
            from = start + prefix.len();
            if from >= line.len() {
                break;
            }
        }
    }
    out
}

/// 去重后的命中描述。
///
/// # 为什么要按位置去重
///
/// `sk-proj-…` **同时**匹配 `sk-proj-`（专门规则）与 `sk-`（通用规则）——
/// 同一个密钥会被报两次。重复报不是"更安全"，而是让命中的行号看起来更多、
/// 干扰开发者定位真实问题。故按字节区间合并：**先到者胜**（`sk-proj-` 在表里
/// 排在 `sk-` 之前，也就是更具体的那个赢）。
fn provider_keys(line: &str) -> Vec<String> {
    let mut spans = provider_spans(line);
    // 长前缀优先：区间更长的先占位，短前缀落在其中的被丢掉
    spans.sort_by_key(|(r, _)| std::cmp::Reverse(r.end - r.start));
    let mut taken: Vec<std::ops::Range<usize>> = Vec::new();
    let mut out = Vec::new();
    for (range, desc) in spans {
        if taken.iter().any(|t| t.start < range.end && range.start < t.end) {
            continue;
        }
        taken.push(range);
        out.push(desc);
    }
    out
}

/// AWS access key id：`AKIA` / `ASIA` + 16 位大写字母数字（共 20）。
///
/// 为什么单独一条：4 字符前缀太短，不配上长度与大小写约束就会在普通英文里误报。
fn aws_access_key(line: &str, out: &mut Vec<(std::ops::Range<usize>, String)>) {
    for prefix in ["AKIA", "ASIA"] {
        let mut from = 0;
        while let Some(pos) = line[from..].find(prefix) {
            let start = from + pos;
            let tail = &line[start + prefix.len()..];
            let upper_alnum: String = tail
                .chars()
                .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                .collect();
            // AWS 的 id 恰好 20 字符；给 16–24 的宽容区间以覆盖历史格式
            if (16..=24).contains(&upper_alnum.len()) {
                let full: String = prefix.chars().chain(upper_alnum.chars()).collect();
                let end = start + full.len();
                if !looks_like_placeholder(&full) {
                    out.push((
                        start..end,
                        format!("`{}…` 形状的 AWS 访问密钥（共 {} 字符）", prefix, full.len()),
                    ));
                }
            }
            from = start + prefix.len();
            if from >= line.len() {
                break;
            }
        }
    }
}

// ── 规则 3：NAME=value 形式的密钥赋值 ─────────────────────────────────────

/// 敏感变量名（规范化后**等于**其一，或**以其结尾**）。
///
/// 为什么用"结尾"而不是"包含"：`api_key_env = "DEEPSEEK_API_KEY"` 里的名字
/// 含 `api_key` 却是**变量名而非密钥**，按"包含"判会误报。按"结尾"判则
/// `api_key_env` 不匹配（它结尾是 `_env`），而 `DEEPSEEK_API_KEY` 匹配。
const SECRET_NAMES: &[&str] = &[
    "api_key",
    "apikey",
    "api_secret",
    "secret",
    "token",
    "auth_token",
    "access_token",
    "refresh_token",
    "password",
    "passwd",
    "passphrase",
    "private_key",
    "access_key",
    "secret_key",
    "client_secret",
    "credential",
    "credentials",
];

/// 名字里**不允许**作为结尾的"非密钥"后缀（否则 `*_key` 这类太宽的名字会误报）。
const NON_SECRET_SUFFIXES: &[&str] =
    &["_path", "_file", "_dir", "_url", "_env", "_name", "_id", "_len", "_count", "_hint"];

/// 识别 `NAME=value` / `NAME: value` / `export NAME=value` / `"NAME": "value"`。
fn env_assignment(line: &str) -> Option<String> {
    let (name, value) = split_assignment(line)?;
    let norm = normalize_name(&name);
    let is_secret_name = SECRET_NAMES.iter().any(|s| norm == *s || norm.ends_with(&format!("_{s}")));
    if !is_secret_name {
        return None;
    }
    if NON_SECRET_SUFFIXES.iter().any(|s| norm.ends_with(s)) {
        return None;
    }
    // 值必须"像个真凭据"：长度够、且不是引用/占位
    let v = trim_quotes(&value);
    if v.chars().count() < 8 {
        return None;
    }
    if looks_like_placeholder(v) || looks_like_reference(v) {
        return None;
    }
    if !value_looks_like_credential(v) {
        return None;
    }
    Some(format!("变量名 `{name}`（值 {} 字符，未回显）", v.chars().count()))
}

/// 把一行拆成 (名字, 值)。认三种分隔：`=`（INI / shell）、`:`（YAML / JSON）。
fn split_assignment(line: &str) -> Option<(String, String)> {
    let t = line.trim().trim_start_matches("export ").trim();
    // 跳过 markdown 引用/列表/注释前缀，让 `> KEY=...`、`- KEY=...` 也能被认出
    let t = t.trim_start_matches(['>', '-', '*', '#']).trim();
    if t.starts_with("//") || t.starts_with("/*") {
        return None;
    }
    let (sep_pos, sep_len) = {
        let eq = t.find('=');
        let colon = t.find(':');
        match (eq, colon) {
            (Some(e), Some(c)) if e < c => (e, 1),
            (Some(_), Some(c)) => (c, 1),
            (Some(e), None) => (e, 1),
            (None, Some(c)) => (c, 1),
            (None, None) => return None,
        }
    };
    let name = trim_quotes(t[..sep_pos].trim()).to_string();
    let value = t[sep_pos + sep_len..].trim().to_string();
    if name.is_empty() || value.is_empty() {
        return None;
    }
    // 名字必须是"标识符形状"（字母数字下划线连字符），否则很可能不是赋值
    //（例如 `见 README: 第 3 节` 这种正文）。
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.') {
        return None;
    }
    Some((name, value))
}

/// 名字规范化：小写、连字符/点统一成下划线（`api-key` 与 `API_KEY` 视为同名）。
fn normalize_name(name: &str) -> String {
    name.trim_matches('"').to_ascii_lowercase().replace(['-', '.'], "_")
}

fn trim_quotes(s: &str) -> &str {
    s.trim().trim_matches(|c| c == '"' || c == '\'' || c == '`')
}

/// 值是否像"被引用而不是字面量"（`$VAR` / `${VAR}` / `env:VAR` / `{{VAR}}`）。
fn looks_like_reference(v: &str) -> bool {
    let t = v.trim();
    t.starts_with('$')
        || t.starts_with("env:")
        || t.starts_with("ENV:")
        || t.starts_with("{{")
        || t.starts_with("<%")
        || t.starts_with("process.env")
        || t.starts_with("os.environ")
        || t.starts_with("os.getenv")
}

/// 值是否"像凭据"：至少要有数字，或者大小写混排。
///
/// 这一条挡的是 `DEEPSEEK_API_KEY`（全大写 + 下划线）这类**标识符**：
/// 它是环境变量的**名字**，不是密钥本身。真实密钥几乎总有数字或大小写混排
/// （base64/hex/带前缀的随机串都是）。
fn value_looks_like_credential(v: &str) -> bool {
    let has_digit = v.chars().any(|c| c.is_ascii_digit());
    let has_upper = v.chars().any(|c| c.is_ascii_uppercase());
    let has_lower = v.chars().any(|c| c.is_ascii_lowercase());
    has_digit || (has_upper && has_lower)
}

// ── 规则 4：URL 内嵌凭据 ──────────────────────────────────────────────────

/// `scheme://user:password@host`。密码段要有一定长度，避免把 `user@host`
/// 这种普通 URL 或 `mailto:` 误判。
fn url_credentials(line: &str) -> Option<String> {
    let mut from = 0;
    while let Some(pos) = line[from..].find("://") {
        let start = from + pos + 3;
        let after = &line[start..];
        // 用户信息段止于第一个 `/`、空白或引号
        let authority_end = after
            .find(|c: char| c == '/' || c.is_whitespace() || c == '"' || c == '\'' || c == '>')
            .unwrap_or(after.len());
        let authority = &after[..authority_end];
        if let Some(at) = authority.rfind('@') {
            let userinfo = &authority[..at];
            if let Some(colon) = userinfo.find(':') {
                let user = &userinfo[..colon];
                let pass = &userinfo[colon + 1..];
                if !user.is_empty() && pass.chars().count() >= 6 && !looks_like_placeholder(pass) && !looks_like_reference(pass) {
                    // 只报用户名与主机，**不报密码**
                    return Some(format!("`{user}:***@…`（密码 {} 字符，未回显）", pass.chars().count()));
                }
            }
        }
        from = start;
        if from >= line.len() {
            break;
        }
    }
    None
}

// ── 占位符判定（第 3 条纪律）─────────────────────────────────────────────

/// 教学用占位符 —— 必须放过，否则 Wiki 会被自己的守卫掏空。
///
/// 两个判据：
/// - **词面**：出现 `your` / `xxx` / `example` / `placeholder` / `redacted` 等，
///   覆盖 `sk-YOUR_KEY_HERE`、`AKIAIOSFODNN7EXAMPLE`（AWS 官方示例）这类；
/// - **字符多样性**：`sk-xxxxxxxxxxxxxxxxxxxx` 只有 1 种字符。
///   真凭据是随机串，重复字符数不会这么低。
fn looks_like_placeholder(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    // ⚠️ 这里**不能**收 `test`：`sk_test_…` 是 Stripe 的真凭据格式，
    // 收了就等于给一整类密钥开后门（本仓的 PROVIDER_PREFIXES 里也列了它，
    // 那种"列出来又亲手滤掉"的自相矛盾正是靠这条注释钉住的）。
    const MARKERS: &[&str] = &[
        "your", "xxx", "placeholder", "redacted", "dummy", "fake", "sample", "todo",
        "changeme", "example", "abc123", "demo", "here", "insert", "replace",
    ];
    if MARKERS.iter().any(|m| lower.contains(m)) {
        return true;
    }
    // 字符多样性：真密钥几乎不可能是"少于 8 种字符的长串"
    let distinct: std::collections::BTreeSet<char> = token.chars().collect();
    if token.chars().count() >= 16 && distinct.len() < 8 {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 命中的**种类**列表（测试只关心种类与行号，不关心文案）。
    fn kinds(text: &str) -> Vec<SecretKind> {
        find_secrets(text).into_iter().map(|h| h.kind).collect()
    }

    fn lines(text: &str) -> Vec<usize> {
        find_secrets(text).into_iter().map(|h| h.line).collect()
    }

    // ── 有牙齿：真格式必须抓住 ────────────────────────────────────────────

    #[test]
    fn pem_private_key_blocks_are_caught() {
        for header in [
            "-----BEGIN RSA PRIVATE KEY-----",
            "-----BEGIN PRIVATE KEY-----",
            "-----BEGIN OPENSSH PRIVATE KEY-----",
            "-----BEGIN EC PRIVATE KEY-----",
            "-----BEGIN PGP PRIVATE KEY BLOCK-----",
        ] {
            let text = format!("前一行\n{header}\nMIIEow...\n");
            assert_eq!(
                kinds(&text),
                vec![SecretKind::PrivateKeyBlock],
                "漏掉了 {header}"
            );
        }
    }

    /// **证书不是私钥**，不能拦 —— 公钥/证书本来就该公开。
    #[test]
    fn public_certificates_are_not_private_keys() {
        for line in [
            "-----BEGIN CERTIFICATE-----",
            "-----BEGIN PUBLIC KEY-----",
            "-----BEGIN SSH2 PUBLIC KEY-----",
        ] {
            assert!(find_secrets(line).is_empty(), "{line} 被误判为私钥");
        }
    }

    #[test]
    fn provider_key_prefixes_are_caught() {
        // 每个都按"前缀 + 足够长的真随机尾部"构造（含数字与大小写混排）
        let samples = [
            "sk-9fK2mQ7dLp3Xr8Tn5Vw1Za6Bc4Ye0Hg",
            "sk-proj-9fK2mQ7dLp3Xr8Tn5Vw1Za6Bc4Ye0Hg7Jd9Ks2Lm",
            "ghp_9fK2mQ7dLp3Xr8Tn5Vw1Za6Bc4Ye0Hg7Jd9K",
            "github_pat_11ABCDEFG0aBcDeFgHiJkLmNoPqRsTuVwXyZ",
            "AKIA3J7QXK2MNPQVWRST",
            "AIzaSyA1bC2dE3fG4hI5jK6lM7nO8pQ9rS0tU",
            "xoxb-9fK2mQ7dLp3Xr8Tn5Vw1Za6B",
            "glpat-9fK2mQ7dLp3Xr8Tn5Vw",
            "npm_9fK2mQ7dLp3Xr8Tn5Vw1Za6Bc4Ye0Hg7Jd9K",
        ];
        for s in samples {
            let hit = find_secrets(s);
            assert_eq!(hit.len(), 1, "漏掉或重复报了 {s}：{hit:?}");
            assert_eq!(hit[0].kind, SecretKind::ProviderKey, "{s}");
        }
    }

    /// 更具体的前缀**优先**，同一个密钥不报两遍（`sk-proj-` 蕴含 `sk-`）。
    #[test]
    fn overlapping_prefixes_do_not_double_report() {
        let text = "key = sk-proj-9fK2mQ7dLp3Xr8Tn5Vw1Za6Bc4Ye0Hg7Jd9Ks2Lm";
        let hits = find_secrets(text);
        assert_eq!(hits.len(), 1, "同一密钥报了 {} 次：{hits:?}", hits.len());
        assert!(hits[0].detail.contains("sk-proj-"), "该由更具体的前缀胜出：{}", hits[0].detail);
    }

    #[test]
    fn env_assignments_with_secret_names_are_caught() {
        for line in [
            "OPENAI_API_KEY=sk-9fK2mQ7dLp3Xr8Tn5Vw1Za6Bc",
            "DEEPSEEK_API_KEY = \"9fK2mQ7dLp3Xr8Tn5Vw1Za6\"",
            "export GITHUB_TOKEN=9fK2mQ7dLp3Xr8Tn5Vw1Za6",
            "aws_secret_access_key: 9fK2mQ7dLp3Xr8Tn5Vw1Za6",
            // 名字不认识（`stripe_key` 不在 SECRET_NAMES 里）但**值**认得出 ——
            // 两条规则互补：名字规则抓"值没特征"的，前缀规则抓"名字随便起"的。
            "stripe_key: sk_live_9fK2mQ7dLp3Xr8Tn5Vw1",
            "PASSWORD=Sup3rS3cret!Value",
        ] {
            assert!(!find_secrets(line).is_empty(), "漏掉了赋值：{line}");
        }
    }

    /// **裸 `_key` 后缀不算敏感名**：`foreign_key` / `sort_key` / `cache_key`
    /// 满地都是，算进来会让守卫天天误报（而误报的守卫下一步就是被关掉）。
    /// 这类名字下若真是凭据，由**值**的前缀规则兜住。
    #[test]
    fn bare_key_suffix_is_not_treated_as_a_secret_name() {
        for line in ["foreign_key = 12345", "sort_key: name", "cache_key = user:42:profile"] {
            assert!(find_secrets(line).is_empty(), "把普通 key 误判成凭据：{line}");
        }
    }

    #[test]
    fn urls_with_embedded_credentials_are_caught() {
        let hit = find_secrets("见 https://alice:hunter2xyz@internal.example.com/db");
        assert_eq!(hit[0].kind, SecretKind::UrlCredentials);
        assert!(hit[0].detail.contains("alice"), "该带上用户名便于定位");
    }

    #[test]
    fn line_numbers_point_at_the_right_line() {
        let text = "正常的一行\n\n第三行有 sk-9fK2mQ7dLp3Xr8Tn5Vw1Za6Bc4Ye0Hg\n";
        assert_eq!(lines(text), vec![3]);
    }

    // ── 不回显：命中描述里不得出现密钥原文（模块纪律第 1 条）─────────────

    /// 这是**最重要的一条测试**：检测器自己不能成为泄露通道。
    ///
    /// 做法：拿一段密钥的**最长公共片段**去反查 —— 命中的每一条描述里，
    /// 都不允许出现密钥的任何连续 6 字符片段。
    #[test]
    fn hit_details_never_echo_the_secret() {
        let secrets = [
            "sk-9fK2mQ7dLp3Xr8Tn5Vw1Za6Bc4Ye0Hg",
            "9fK2mQ7dLp3Xr8Tn5Vw1Za6Bc4Ye0Hg7Jd9K",
            "Sup3rS3cret!ValueXYZ",
            "hunter2xyz",
        ];
        let text = format!(
            "OPENAI_API_KEY={}\nghp_{}\nPASSWORD={}\nhttps://alice:{}@host/db\n",
            secrets[0], secrets[1], secrets[2], secrets[3]
        );
        let hits = find_secrets(&text);
        assert!(!hits.is_empty(), "这段文本必须命中，否则本测试没在测东西");
        for h in &hits {
            for s in secrets {
                for w in s.as_bytes().windows(6) {
                    let frag = std::str::from_utf8(w).unwrap();
                    assert!(
                        !h.detail.contains(frag),
                        "命中描述回显了密钥片段 {frag:?}：{}",
                        h.detail
                    );
                }
            }
        }
    }

    // ── 无牙齿的另一半：占位符与普通文本必须放过 ─────────────────────────
    //
    // 这一半同样重要：天天误报的守卫下一步就是被关掉（模块纪律第 3 条）。

    #[test]
    fn documentation_placeholders_are_not_flagged() {
        for text in [
            "OPENAI_API_KEY=sk-YOUR_KEY_HERE",
            "api_key=sk-xxxxxxxxxxxxxxxxxxxxxxxx",
            "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE",
            "Authorization: Bearer ${GITHUB_TOKEN}",
            "api_key = $OPENAI_API_KEY",
            "token: <your-token-here>",
            "SECRET=changeme",
            "password = os.environ[\"DB_PASSWORD\"]",
            "密码见 `docs/setup.md`",
        ] {
            assert!(find_secrets(text).is_empty(), "误报了占位/引用：{text}");
        }
    }

    /// **变量名不是密钥**：`api_key_env = "DEEPSEEK_API_KEY"` 指的是"去读哪个
    /// 环境变量"，值本身是标识符而不是凭据 —— 本仓自己的 `providers.json`
    /// 就是这个形状，误报会让 Wiki 把自己的文档拦下。
    #[test]
    fn names_of_secret_variables_are_not_secrets() {
        for line in [
            "api_key_env = \"DEEPSEEK_API_KEY\"",
            "\"apiKeyEnv\": \"OPENAI_API_KEY\"",
            "token_path = \"/etc/neo/token\"",
            "secret_file = \"~/.neo/keys.json\"",
            "api_key_id = \"abc123def456\"",
        ] {
            assert!(find_secrets(line).is_empty(), "把变量名误当成了密钥：{line}");
        }
    }

    /// 普通正文里的 `Key:` / `Token:` / `Password:` 是**词**，不是凭据。
    #[test]
    fn prose_mentioning_secret_words_is_not_flagged() {
        for line in [
            "## API Key 的申请方式",
            "Token: 见上一节",
            "Password: 请向管理员索取",
            "把 token 放进环境变量，不要写进代码",
            "密钥长度不足 8 位时直接拒绝",
        ] {
            assert!(find_secrets(line).is_empty(), "散文被误判：{line}");
        }
    }

    #[test]
    fn ordinary_urls_are_not_flagged() {
        for line in [
            "https://example.com:8080/path",
            "git@github.com:proteus-vue/proteus-code.git",
            "mailto:someone@example.com",
            "ssh://git@example.com/repo",
        ] {
            assert!(find_secrets(line).is_empty(), "普通 URL 被误判：{line}");
        }
    }

    /// 字符多样性判据：长但那怕只有一种字符的长串是模板，不是凭据。
    #[test]
    fn long_but_low_variety_tokens_are_placeholders() {
        assert!(find_secrets("token=aaaaaaaaaaaaaaaaaaaaaaaa").is_empty());
        assert!(find_secrets("sk-0000000000000000000000000000").is_empty());
    }

    /// 反面自证：**同一形状**换成真随机串就必须报出来 —— 否则上面那些
    /// "不报"的测试可能只是因为规则整体失效了。
    #[test]
    fn the_same_shapes_with_random_bodies_do_get_flagged() {
        assert!(!find_secrets("token=9fK2mQ7dLp3Xr8Tn5Vw1").is_empty());
        assert!(!find_secrets("sk-9fK2mQ7dLp3Xr8Tn5Vw1Za6Bc4Ye0Hg").is_empty());
        // 长度不足的 `sk-` 串**不报**（那是文档里常见的通配写法），
        // 这条钉住阈值本身 —— 它是"少误报"与"不漏报"的分界线。
        assert!(find_secrets("sk-9fK2mQ7dLp3Xr8Tn5Vw1Za6").is_empty());
    }

    #[test]
    fn empty_and_plain_text_is_clean() {
        assert!(find_secrets("").is_empty());
        assert!(find_secrets("这是一份普通的说明文档。\n没有任何密钥。").is_empty());
    }
}
