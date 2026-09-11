# 上手与排错

## 环境

| 需求 | 版本 | 说明 |
|---|---|---|
| Node.js | ≥ 22.19 | DSH 引擎要求。系统 Node 可以是 18——桌面应用会用 nvm 或 Electron 自带 Node |
| pnpm | ≥ 9 | workspace 与 DSH profile 的包管理器 |
| 操作系统 | macOS / Windows | Linux 不是 DSH 官方桌面发布目标 |

## 安装

```bash
cd proteus-code
pnpm install
```

安装会下载 Electron（约 100 MB）、DSH 及其依赖。首次安装允许以下 native 构建脚本（已预置在 `pnpm-workspace.yaml`）：`esbuild`、`electron`、`koffi`、`node-pty`、`protobufjs`、`@deepseek-ai/dsh-subprocess-local`。

## 运行

```bash
pnpm start      # 桌面窗口
pnpm web        # 无 GUI，起本地 Web 版并打印 URL
pnpm smoke      # 无人值守：截图到 .proteus-code/smoke.png
pnpm verify     # 完整验证：构建 + 测试 + typecheck + harness 启动
```

### 第一次启动会发生什么

应用拥有自己的 Harness home（默认 `~/.proteus-code`），与你已有的 `dsh` CLI 状态完全隔离。首次启动：

```
[proteus-code] repo:    /path/to/proteus-code
[proteus-code] home:    /Users/you/.proteus-code
[proteus-code] initializing the desktop Harness profile…
[proteus-code] installing the Proteus bundle into the profile…
[proteus-code] UI ready at http://127.0.0.1:<随机端口>/?token=…
```

之后每次启动会跳过 profile 初始化；只有当你重新构建了 bundle（`dist/index.js` 比 profile 里的副本新）时，才会自动 remove + add 刷新。

## 让 agent 操作你的 Proteus 工程

在应用里选择工作区时，指向一个 Proteus 工程目录即可——bundle 会从该目录向上自动发现工程。若自动发现失败，显式配置：

```yaml
# ~/.proteus-code/profiles/proteus-code/cordis.patch.yml（用户覆盖层）
- id: proteus-code
  name: '@proteus-code/dsh-proteus-code'
  config:
    proteusRoot: /absolute/path/to/proteus
    timeoutMs: 180000
```

> 注意：patch 是**按行整体替换 config**，不是深合并，所以覆盖时要写全你要保留的字段。

## 配置执行策略（可选）

在工程里放一个 `.proteus-code/policy.json`，就能给 shell 命令加上「允许 / 询问 / 拒绝」规则。向上自动发现，改动即时生效（按 mtime 热更新）。

```json
{
  "rules": [
    {
      "id": "no-recursive-delete",
      "pattern": ["rm", ["-rf", "-fr", "-r"]],
      "decision": "forbidden",
      "justification": "递归删除在本工作区被禁止；请删除具体路径。",
      "match": ["rm -rf build"],
      "notMatch": ["rm file.txt"]
    },
    {
      "id": "gate-push",
      "pattern": ["git", "push"],
      "decision": "prompt",
      "justification": "推送会发布内容，请确认远端与分支。"
    },
    {
      "pattern": ["npm", "publish"],
      "decision": "forbidden",
      "justification": "发布由 CI 负责。改用 `npm version` + PR。"
    }
  ]
}
```

要点：

- **`pattern`** 是命令开头的有序 token；某一项写成数组表示备选。
- **`decision`** 为 `allow`（不额外限制）/ `prompt`（弹审批）/ `forbidden`（直接拦）；默认 `allow`。
- **`match` / `notMatch`** 是这条规则自带的测试，加载时校验——写错了应用会**启动失败并告诉你哪条规则错**，不会静默放行。
- 命中多条规则时**最严格者胜出**。
- `allow` **不会绕过**宿主的沙箱与审批策略，只是「策略不额外限制」。

拦截时你会看到形如：

```
Blocked by proteus-code policy (rule: no-recursive-delete). 递归删除在本工作区被禁止；请删除具体路径。
Command: rm -rf build
```

### 让 agent 先自查

模型可以用 `proteus_policy` 工具在不执行的情况下问「这条命令会怎样」：

```
proteus_policy({ command: "git push origin main" })
→ decision: prompt
  matched: gate-push
  justification: 推送会发布内容，请确认远端与分支。
```

### 钉死可执行文件路径

```json
{
  "hostExecutables": [{ "name": "git", "paths": ["/usr/bin/git", "/opt/homebrew/bin/git"] }]
}
```

配合 config 的 `resolveHostExecutables: true`，`git` 的 basename 规则只对列出的绝对路径生效——`/tmp/evil/git` 无法继承。

> 不写这个策略文件时不会有任何行为变化，一切仍由 DSH 自身的沙箱与审批策略决定。

## 环境变量

| 变量 | 作用 |
|---|---|
| `PROTEUS_CODE_HOME` | 覆盖 Harness home（默认 `~/.proteus-code`） |
| `PROTEUS_CODE_REPO` | 覆盖本项目仓库根（默认从应用目录向上探测） |
| `PROTEUS_CODE_NODE` | 指定 Node ≥22 可执行文件 |
| `PROTEUS_CODE_DSH` | 指定 DSH CLI 入口脚本 |
| `PROTEUS_CODE_SMOKE` | 设为 PNG 路径则进入自检模式：加载后截图并退出 |
| `PROTEUS_CODE_SMOKE_WAIT_MS` | 自检等待时长（默认 20000ms） |
| `PROTEUS_CODE_TEST_CHECKOUT` | 单元测试加入真实 Proteus 集成用例 |
| `DSH_HOME` | DSH 原生变量；桌面应用会把它设为自己的 home |

## 排错

### 启动报 `no Node ≥22 runtime found`

系统 `node` 是 18 且没有可用的 nvm 版本，同时不在 Electron 里。装一个 Node 22+，或：

```bash
export PROTEUS_CODE_NODE=/path/to/node22/bin/node
```

### 启动报 `the Proteus bundle is not built`

```bash
pnpm build:bundle
```

### 报 `Cannot find package '@deepseek-ai/<某包>'`（插件装载失败）

说明 bundle 的安装方式退化成了 symlink。确认 profile 的 `package.json` 里依赖写的是 `file:` 而不是裸路径：

```json
"@proteus-code/dsh-proteus-code": "file:/path/to/proteus-code/packages/proteus-code-bundle"
```

修复后删掉 profile 重新启动：

```bash
rm -rf ~/.proteus-code/profiles/proteus-code
pnpm start
```

原因见 [architecture.md 的「依赖解析」](architecture.md#依赖解析本项目最不显然的一点)。

### `ERR_PNPM_FETCH_404 … dsh-type-meta`

你手动把 DSH 包加进了某个 `dependencies`/`peerDependencies`。上游该 peer 目标未发布，必须只在 `devDependencies` 里放 DSH 包。详见 [architecture.md](architecture.md#为什么-dsh-包只能进-devdependencies)。

### 界面一直停在 “Loading plugins…”

客户端插件还在装载。若超过 ~30 秒仍不动，用 `pnpm smoke` 拿到诊断——它会把控制台错误与失败请求一起写进 `<shot>.png.txt`。常见原因是某个客户端插件包缺失或 URL 404。

### 界面品牌仍是 deepseek HARNESS

说明客户端 bundle 没被装载或官方品牌行没被禁用。检查：

1. `packages/proteus-code-bundle/dist/client.js` 存在
2. `package.json` 有 `dsh.client` 声明
3. `cordis.patch.yml` 里 `ui-brand-official: disabled: true` 生效

用 `dsh --profile proteus-code --dump-config | grep -A3 ui-brand` 确认禁用已合并。

### 想看不带本项目的原始 DSH 界面做对比

```bash
DSH_HOME=/tmp/dsh-plain node node_modules/@deepseek-ai/dsh/lib/bin.js web
```

## 开发循环

```bash
# 改 host 插件 / 工具 / 命令 / 技能
pnpm build:bundle
pnpm --filter @proteus-code/desktop web     # 或 pnpm start

# 改 Electron 主进程 / profile 逻辑
pnpm --filter @proteus-code/desktop build
pnpm start

# 只跑单测
pnpm test
```

改 bundle 后必须重启应用：profile 的 bundle 集合只在启动时读取（DSH 的启动边界），运行中重载的是 patch 文件而不是 bundle 成员。
