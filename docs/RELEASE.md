# 发布检查清单

> 面向维护者。目标读者拿这份文档能**从头走完一次发布**，含首次（bootstrap）
> 与后续（纯打 tag）两种情况。
>
> 设计依据与踩过的坑见 `PROJECT_MEMORY.md §0 分发与发布`。

---

## 0. 先理解版本真源（最容易出错的地方）

版本在两个地方出现，**来源不同**：

| 位置 | 来源 | 谁在用 |
|---|---|---|
| `Cargo.toml` → `[workspace.package] version` | **真源**，手工改 | 二进制自报的 `neo --version`；crates 元数据 |
| npm 包版本 | 由 **git tag** 注入（`v0.1.0` → `0.1.0`） | `npm install` 装的版本 |
| GitHub Release / 产物文件名 | 同上，取自 tag | 下载的 tar 名 |

三者**必须一致**。`scripts/publish-npm.sh` 会在发布前硬校验
Cargo.toml 与传入版本是否相同，不一致直接失败——但 GitHub Release 的
产物名不会校验，所以**发版前先确认 tag 没有打错**。

---

## 1. 首次发布：bootstrap

包在 npm 上还不存在时，**无法**配置 Trusted Publisher（它只在包的设置页里），
所以第一次必须用 Granular access token 发出去。这是官方文档没讲的鸡生蛋问题。

### 1.1 准备版本与提交

```bash
# ① 改版本真源
$EDITOR Cargo.toml          # [workspace.package] version = "0.1.0"

# ② 刷新 Cargo.lock（workspace 成员版本变了，锁文件要跟着更新）
cargo metadata --format-version 1 >/dev/null

# ③ 跑门禁，确认全绿再发
CARGO_NET_OFFLINE=true bash scripts/verify.sh

# ④ 提交（含 Cargo.toml 与 Cargo.lock）
git add Cargo.toml Cargo.lock && git commit -m "chore(release): v0.1.0"
```

### 1.2 建 Granular access token（npm 侧）

Automation token 已于 2025-11 被 npm 移除，只有 Granular。

1. 登录 [npmjs.com](https://www.npmjs.com) → 右上角头像 → **Access Tokens**
2. **Generate New Token**
3. Token name：`github-actions-publish`
4. ✅ **勾选 Bypass two-factor authentication**

   > 这一步对 CI 是**必需**的。npm 规定发布要么有账号 2FA、要么用一个
   > "bypass 2FA enabled" 的 granular token；CI 里没人能输 OTP，所以不勾选
   > 会发布失败。
   > 补充：2026-08 起 bypass-2FA token 不能再做"账号身份/治理类"操作，
   > 但**发布包不受影响**。
   > 若账号或组织**强制** 2FA 导致这一项不可用，就只能走第 2 节的
   > Trusted Publishing（OIDC 不受此限）。

5. **Packages and scopes** → 选 **Read and write**
   （不要选 "stage only"，它不能直接发布）
6. **Select Packages** → All Packages
7. **Expiration** → 选一个期限（至少 1 天后）
8. Generate Token → **立刻复制**（只显示一次）

### 1.3 存成 GitHub secret

仓库 → **Settings** → **Secrets and variables** → **Actions** → **Secrets** 页
→ **New repository secret**：

- Name：`NPM_TOKEN`
- Secret：粘贴上一步的 token

### 1.4 打 tag 触发发布

> ⚠️ **推 tag 会立刻创建公开的 GitHub Release**（构建三平台二进制并上传，
> 用仓库默认 `GITHUB_TOKEN`，不需要额外密钥）。这是对外可见、难以当作
> "没发生"的动作 —— 别拿它当试验。想先验证，走 §4 的本地演练。

```bash
git push origin main
git tag v0.1.0
git push origin v0.1.0
```

`release.yml` 会依次做：构建三平台二进制 → 传 GitHub Release →
打 npm 包并发布（平台包在前、主包在后）。

> 若此时还没配任何 npm 凭证，npm 这步会**告警跳过**，Release 产物照常
> 上传 —— 也就是说可以先发 Release、之后再补 npm 发布。

### 1.5 验证

```bash
# npm 上应有 4 个包（1 个主包 + 3 个平台包）
npm view neo-code version
npm view neo-code-darwin-arm64 version

# 真装一遍（注意用临时 prefix，别污染本机）
npm install -g --prefix /tmp/neo-check neo-code
/tmp/neo-check/bin/neo --version     # 应输出 neo 0.1.0
rm -rf /tmp/neo-check
```

---

## 2. 切到 Trusted Publishing（首次发布成功后立刻做）

OIDC 可信发布**不需要任何长期凭证**，且 npm 计划 2027-01 移除 granular token
的直接发布权限，所以这是长期方案。

### 2.1 npm 侧：给 4 个包各配一次

对 **`neo-code`、`neo-code-darwin-arm64`、`neo-code-darwin-x64`、
`neo-code-linux-x64`** 逐个操作：

进入包页面 → **Settings** → **Trusted Publisher** → 选 **GitHub Actions**，填：

| 字段 | 值 |
|---|---|
| Organization or user | `proteus-vue` |
| Repository | `proteus-code` |
| Workflow filename | `release.yml` |
| Environment | **留空** |

> 文件名**大小写敏感**且必须与 `.github/workflows/` 下的完全一致。
> Environment 留空是因为 `publish-npm` job 没有声明 environment；
> 若在 npm 侧填了，job 就必须加 `environment:`，否则认证失败。

### 2.2 GitHub 侧：打开开关

仓库 → Settings → Secrets and variables → Actions → **Variables** 页（不是 Secrets）
→ New repository variable：

- Name：`NPM_TRUSTED_PUBLISHING`
- Value：`true`

### 2.3 **删掉 NPM_TOKEN**（关键，别漏）

仓库 → Settings → Secrets and variables → Actions → Secrets →
删掉 `NPM_TOKEN`。

> 为什么必须删：`NODE_AUTH_TOKEN` 存在时 npm **优先用 token、绕过 OIDC**。
> 留着它，表面配置成功，实际仍在用长期凭证——等于没切换。

---

## 3. 后续发布（纯打 tag）

1. 改 `Cargo.toml` 版本 → `cargo metadata --format-version 1 >/dev/null` 刷新锁
2. `bash scripts/verify.sh` 全绿
3. 提交并推送
4. `git tag vX.Y.Z && git push origin vX.Y.Z`
5. 按 §1.5 验证

`publish-npm.sh` 是幂等的：已发布过的版本会跳过，所以重复推同一个 tag
不会报错，也不会重复发布。

---

## 4. 演练（不发布）

想先确认打包没问题，用 dry-run（会真打出包并本地装载冒烟，只是不上传）：

```bash
# 先准备产物目录：模拟 release.yml 的输出
mkdir -p /tmp/dist && cargo build --release -p neo-code-cli
D=/tmp/dist/neo-v0.1.0-aarch64-apple-darwin && mkdir -p "$D"
cp target/release/neo "$D/" && cp README.md LICENSE "$D/"
tar -C /tmp/dist -czf /tmp/dist/neo-v0.1.0-aarch64-apple-darwin.tar.gz "$(basename $D)"

bash scripts/publish-npm.sh --dist /tmp/dist --version 0.1.0 --dry-run
```

---

## 5. 出问题怎么查

| 现象 | 原因 | 处理 |
|---|---|---|
| `EOTP` / 要求 OTP | granular token 没勾 Bypass 2FA | 重建 token 并勾选；或改用 Trusted Publishing |
| `E403` 无权限发布 | token 权限不是 "Read and write"，或过期 | 重建 token |
| `ENEEDAUTH` | secret 没配、或名字不是 `NPM_TOKEN` | 核对 secret 名 |
| `E404` 找不到包 | 首次发布时平台包还没存在 | 正常——脚本先发平台包再发主包，按序即可 |
| 版本漂移报错 | tag 与 `Cargo.toml` 不一致 | 按报错提示统一两者 |
| workflow 里 npm 步骤被跳过 | 既没有 `NPM_TOKEN` 也没有 `NPM_TRUSTED_PUBLISHING` | 见 §1.3 或 §2.2 |
| Trusted Publishing 认证失败 | workflow 文件名不符 / Environment 不一致 / 用了自托管 runner | 逐项核对 §2.1；自托管 runner 不支持 |
| Release 产物正常但 npm 没动 | npm 步骤被 if 条件跳过（预期行为，不会让整体失败） | 看该 job 的 warning 注解 |

---

## 6. 撤销一个已发布的版本

npm 的规则很硬：**已被任何项目安装过的版本不能 `unpublish`**。

```bash
# 方案 A：废弃（推荐）—— 包仍在，但安装时告警
npm deprecate neo-code@0.1.0 "该版本有严重缺陷，请升级到 0.1.1"

# 方案 B：发布修复版（正路）
#   改版本 → 打新 tag → 正常发布

# 方案 C：72 小时内且无人依赖，才可能删掉
npm unpublish neo-code@0.1.0
```

发布前跑一遍 §4 的演练，比发布后补救便宜得多。
