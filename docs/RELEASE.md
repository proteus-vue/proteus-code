# 发布检查清单

> 面向维护者。目标读者拿这份文档能**从头走完一次发布**，含首次（bootstrap）
> 与后续（changeset 流程）两种情况。
>
> 设计依据与踩过的坑见 `PROJECT_MEMORY.md §0 分发与发布`。

---

## 0. 先理解版本从哪来（最容易出错的地方）

**版本号不要手工改**（首次发布除外）。它由 changeset 流程从 `.changeset/`
碎片自动推进：

```
你写一条碎片 → 合并到 main → 机器人开 Version PR（推进版本 + 汇总 CHANGELOG）
→ 你合并那个 PR（这一步才打 tag、构建、发布）
```

三个地方会出现版本，**全部由机器同步**：

| 位置 | 谁写 | 谁在用 |
|---|---|---|
| `Cargo.toml` → `[workspace.package] version` | `changeset.py version` | 二进制自报的 `neo --version` |
| 各 crate 清单里**内部依赖**的 `version = "..."` | 同上 | cargo 解析（`^0.x` 不匹配 `0.y`，必须同步，否则构建失败） |
| `Cargo.lock` / `CHANGELOG.md` / npm `package.json` | 同上 | 打包与发布 |

npm 包与 GitHub Release 的版本取自 **git tag**，而 tag 由流程从 Cargo 版本生成，
因此天然一致。`scripts/publish-npm.sh` 另有一道硬校验兜底：传入版本与
Cargo.toml 不符就直接失败。

> 为什么碎片只写 bump 级别、不写包名：本仓库是**单一版本工作区**，
> 23 个 crate 共享一个版本号。

---

## 1. 首次发布：bootstrap

包在 npm 上还不存在时，**无法**配置 Trusted Publisher（它只在包的设置页里），
所以第一次必须用 Granular access token 发出去。这是官方文档没讲的鸡生蛋问题。

### 1.1 准备版本与提交

首次发布时还没有 changeset 流程可用（`CHANGELOG.md` 都还不存在），
所以这一次**手工**定版本：

```bash
# ① 改版本真源
$EDITOR Cargo.toml          # [workspace.package] version = "0.1.0"

# ② 同步各 crate 内部依赖的 version（若有）—— 也可以用脚本一次做完：
python3 scripts/changeset.py version      # 需要先放一条碎片；或手工改

# ③ 刷新 Cargo.lock
cargo metadata --format-version 1 >/dev/null

# ④ 跑门禁，确认全绿再发
CARGO_NET_OFFLINE=true bash scripts/verify.sh

# ⑤ 提交（含所有清单与 Cargo.lock）
git add -A && git commit -m "chore(release): v0.1.0"
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

## 3. 日常发布：写 changeset，不要手工改版本

版本号**不要手改**。日常流程是"写一条碎片 → 机器人推进版本 → 你合并"：

### 3.1 在你的改动 PR 里加一条碎片

```bash
python3 scripts/changeset.py new --bump minor --note "新增 npm 分发"
```

（级别怎么选见 `.changeset/README.md`。）CI 的 `changeset` 门禁会检查
产品面改动（`crates/`、`npm/`）有没有带碎片；纯重构/CI/文档给 PR 打
`no-changeset` 标签即可豁免。

### 3.2 合并后机器人开 Version PR

推到 `main` 后，`changeset-release` workflow 会开一个
**`chore(release): vX.Y.Z`** 的 PR，内容 = 推进版本 + 汇总 CHANGELOG +
删掉已消费的碎片。它每次从 `main` 重建，所以永远等于 `main + 一次版本推进`。

### 3.3 合并 Version PR —— 这一步才发布

合并后同一个 workflow 再次运行，此时已无碎片，于是：

1. 给当前版本打 tag `vX.Y.Z` 并推送
2. **显式触发** `release.yml`（见下方"为什么需要显式触发"）
3. 构建三平台二进制 → GitHub Release → 发布 npm

`CHANGELOG.md` 由机器维护，不要手改它里面自动生成的段落。

> **为什么需要"显式触发"**：GitHub 规定用默认 `GITHUB_TOKEN` 做的事件
> **不会**再创建新的 workflow run（防止递归）。所以机器人推的 tag
> **不会**触发 `release.yml`。而 `workflow_dispatch` 是该规则的两个例外之一，
> 因此机器人推完 tag 后主动 dispatch —— 这是确定性做法，且不需要额外 PAT。

### 3.4 手动发布（特殊情况下）

确实需要手工发某个版本时：

```bash
# 用脚本推进版本（会同步内部依赖版本、Cargo.lock、CHANGELOG、npm 清单）
python3 scripts/changeset.py version
git add -A && git commit -m "chore(release): vX.Y.Z"
git tag vX.Y.Z && git push origin main --tags
# 若 tag 没有触发 release（或想重跑），手动 dispatch：
gh workflow run release.yml --ref vX.Y.Z
```

`publish-npm.sh` 是幂等的：已发布过的版本会跳过，所以重复推同一个 tag
或重跑 workflow 不会报错、也不会重复发布。

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

**先看 `publish-npm` 这个 job**。Release 成功 ≠ npm 发布成功——两者是并行的
job，npm 那一步可能被跳过而整体仍显示绿色。

| 现象 | 原因 | 处理 |
|---|---|---|
| **Release 全绿但 npm 上什么都没有** | `publish-npm` job 判定无凭证而跳过了发布 | 现在这一步会**直接失败**并打印诊断表（见下方"最常踩的坑"）。看该 job 的日志 |
| 日志说"没有任何可用的 npm 凭证" | token 加成 Variable 而非 Secret（最常见） | Settings → Secrets and variables → Actions → **Secrets** 页放 `NPM_TOKEN`；仅用无密钥方案则改用 §2 |
| `EOTP` / 要求 OTP | granular token 没勾 Bypass 2FA | 重建 token 并勾选；或改用 Trusted Publishing |
| `E403` 无权限发布 | token 权限不是 "Read and write"，或已过期 | 重建 token |
| `ENEEDAUTH` | 凭证没传到 npm | 核对 secret 名恰为 `NPM_TOKEN` |
| `E404` 找不到包 | 首次发布时平台包尚未存在 | 正常——脚本先发平台包再发主包，按序即可 |
| `E409` 版本已存在 | 该版本发过了 | 幂等处理：脚本会跳过；要重发就升版本 |
| 版本漂移报错 | 传入版本与 `Cargo.toml` 不符 | 按报错提示统一两者（正常流程不会发生） |
| Trusted Publishing 认证失败 | workflow 文件名不符 / Environment 不一致 / 自托管 runner | 逐项核对 §2.1；自托管 runner 不支持 |
| 加了凭证但不想在本次发 | —— | 把仓库变量 `NPM_SKIP_PUBLISH` 设为 `true` 可显式跳过 |

### 最常踩的坑：token 加成了 Variable

这次就踩过。`release.yml` 找的是 **Secret** `NPM_TOKEN`，而它被加在了
**Variables** 页 —— 于是 job 判定无凭证、跳过发布，而整体仍是绿色，
表现为"推送后没发布但不报错"。

**修好凭证后怎么重跑**（二选一）：

```bash
# 推荐：以 tag 为 ref 重新触发一次。版本从 Cargo.toml 读，不依赖 event ref，
# 因此这条路径与 tag 推送等价（会建 Release、会发 npm）。
gh workflow run release.yml --ref v0.1.0
```

或在网页上：**Actions → release → Run workflow**，把 "Use workflow from"
选成 tag `v0.1.0`。

> ⚠️ **不要用旧 run 的 "Re-run all jobs"**：它跑的是**该 tag 处那份旧
> workflow 文件**（还带着静默跳过的 bug），且不会重新读取你刚补的凭证配置
> —— 看起来重跑了一遍，其实什么都不会发生。必须在**包含修复的 main** 上
> 以 tag 为 ref 重新触发。

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
