# @proteus-code/dsh-proteus-code

Proteus 的 DeepSeek Harness bundle：把 Proteus CLI、斜杠命令与领域技能接入 DSH agent，并替换应用品牌。

## 它贡献什么

- **9 个 `proteus_*` CLI 工具** —— `health` / `check` / `build` / `explain` / `rules` / `audit` / `conformance` / `migrate_mp` / `cli`，每个都是真实 `proteus` 子命令的封装
- **1 个策略工具** —— `proteus_policy`，不执行命令即可预测某条命令会被允许/询问/拒绝
- **执行策略引擎** —— 挂在 `tools/pre-execute` 上的前缀规则引擎（移植自 OpenAI Codex 的 `codex-execpolicy`）
- **5 个斜杠命令** —— `/proteus-health`、`/proteus-check`、`/proteus-build`、`/proteus-rules`、`/proteus-explain`
- **1 个 runtime 技能** —— `proteus-development`
- **整站界面重构** —— 90 个主题 token（79 `--dsw-alias-*` + 11 `--dsw-specific-*`，明暗两套）、品牌 slot 组件、标题 / favicon / 启动页字标 / 全局 CSS
- **液态玻璃主视觉** —— 全窗极光渐变 + `backdrop-filter` 毛玻璃 + SVG 位移滤镜真实折射 + 镜面高光灯边缘；带无障碍与性能降级

## 双面包

| 入口 | 目标 | 贡献 |
|---|---|---|
| `dist/index.js` | Node（宿主） | 工具 / 命令 / 技能 / 策略守卫 / index.html 品牌注入 |
| `dist/client.js` | 浏览器（渲染器） | 主题 token / 品牌 slot / 标题 / 文案覆盖（`dsh.client`） |

`@deepseek-ai/*` 在构建时保持 external，由 harness 提供；类型检查使用 `src/dsh-host.d.ts` 的本地声明。

## 安装

```bash
dsh plugin --profile <name> add -w file:/path/to/packages/proteus-code-bundle
```

必须用 `file:`（不能用裸路径），否则 pnpm 会装成 symlink，插件将无法解析 DSH 包。详见仓库 `docs/architecture.md`。

## 配置

```yaml
- id: proteus-code
  name: '@proteus-code/dsh-proteus-code'
  config:
    proteusRoot: /path/to/proteus    # 省略则从 cwd 向上自动发现
    proteusCommand: proteus          # 覆盖 CLI 调用方式
    timeoutMs: 120000
    enableCommands: true
    enableSkill: true
    # 执行策略（可选）：内联规则，或让插件发现 .proteus-code/policy.json
    policy:
      rules:
        - pattern: ['rm', ['-rf', '-fr']]
          decision: forbidden
          justification: '改用具体路径删除。'
    discoverPolicy: true             # 默认 true
    resolveHostExecutables: false    # 开启后允许绝对路径回退 basename 规则
    enableLiquidGlass: true          # 液态玻璃层；false 则保留其余品牌改造
```

策略只**收紧**不放松：`allow` 是委托，宿主的沙箱与审批策略始终生效。

## 边界

- DSH 处于 developer preview，契约可能变更
- 工具执行优先走 `ctx.shell`，因而受会话沙箱与审批策略约束
- `proteus_cli` 兜底工具只接受已知 Proteus 子命令
- 文案覆盖层只在 locale 命名空间不可重新注册时使用，清单锁定为 1 条
