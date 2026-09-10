/**
 * proteus-code — the Proteus profile bundle for DeepSeek Harness.
 *
 * Contributes the Proteus CLI tool surface, human slash commands, and the
 * Proteus development skill to whichever profile mounts this bundle. The
 * deployment persona is set declaratively by `cordis.patch.yml`, not here.
 *
 * @module @proteus-code/dsh-proteus-code
 */

import type { Context } from '@deepseek-ai/cordis'
import Schema from '@deepseek-ai/schemastery'
import { installBrandAssets } from './brand-assets.ts'
import { registerProteusCommands, type CommandsSeam } from './commands.ts'
import { installPolicyGuard, type HostContext } from './policy-guard.ts'
import { createPolicyRuntime } from './policy-runtime.ts'
import { createPolicyTool } from './policy-tool.ts'
import type { ShellSeam } from './proteus-cli.ts'
import { registerProteusSkill, type SkillsSeam } from './skill.ts'
import { createProteusTools, type ToolsConfig } from './tools.ts'

export const name = 'proteus-code'

/** The tool registry is the one hard dependency; everything else is optional. */
export const inject = ['tools']

/** Plugin configuration, supplied through the bundle patch and user overlays. */
export interface Config {
  /** Absolute path to a Proteus checkout. When omitted, it is discovered by walking up from the session working directory. */
  proteusRoot?: string
  /** Full override for how the Proteus CLI is invoked, e.g. `proteus` or `node /path/to/cli.js`. */
  proteusCommand?: string
  /** Per-invocation timeout in milliseconds. */
  timeoutMs?: number
  /** Register the `/proteus-*` slash commands. Defaults to true. */
  enableCommands?: boolean
  /** Register the Proteus development skill. Defaults to true. */
  enableSkill?: boolean
  /** Rebrand the served HTML (title, favicon, brand CSS). Defaults to true. */
  enableBrandAssets?: boolean
  /** Inject the liquid-glass surface layer (backdrop blur, rim light, lens refraction). Defaults to true. */
  enableLiquidGlass?: boolean
  /** Replace the hero workspace picker with one that also offers "work without a project". Defaults to true. */
  workspacePicker?: boolean
  /** Execution-policy rules, appended to any discovered policy file. */
  policy?: unknown
  /** Explicit path to a policy file; disables discovery when set. */
  policyFile?: string
  /** Whether to look for `.proteus-code/policy.json` upward from the working directory. Defaults to true. */
  discoverPolicy?: boolean
  /** Allow absolute program paths to fall back to basename rules, constrained by `hostExecutables`. */
  resolveHostExecutables?: boolean
}

export const Config: Schema<Config> = Schema.object({
  proteusRoot: Schema.string(),
  proteusCommand: Schema.string(),
  timeoutMs: Schema.number().default(120_000),
  enableCommands: Schema.boolean().default(true),
  enableSkill: Schema.boolean().default(true),
  enableBrandAssets: Schema.boolean().default(true),
  enableLiquidGlass: Schema.boolean().default(true),
  workspacePicker: Schema.boolean().default(true),
  policy: Schema.any(),
  policyFile: Schema.string(),
  discoverPolicy: Schema.boolean().default(true),
  resolveHostExecutables: Schema.boolean().default(false),
})

export function apply(ctx: Context, config: Config): void {
  const toolsConfig: ToolsConfig = {
    proteusRoot: config.proteusRoot,
    proteusCommand: config.proteusCommand,
    timeoutMs: config.timeoutMs ?? 120_000,
  }
  // `ctx.shell` is present in every base-backed profile; absent profiles get the
  // local child_process fallback so the tools still function.
  const shell = ctx.get('shell') as ShellSeam | undefined
  const cwd = () => process.cwd()

  for (const tool of createProteusTools({ config: toolsConfig, shell, cwd })) {
    ctx.tools.register(tool)
  }

  // One runtime owns policy resolution, so the guard and the introspection tool
  // cannot disagree about what the policy says.
  const policyRuntime = createPolicyRuntime(
    {
      policy: config.policy,
      policyFile: config.policyFile,
      discoverPolicy: config.discoverPolicy,
      resolveHostExecutables: config.resolveHostExecutables,
    },
    cwd,
  )
  ctx.tools.register(createPolicyTool(policyRuntime))
  installPolicyGuard(ctx as unknown as HostContext, { runtime: policyRuntime })

  // Product identity in the served HTML (title, favicon, brand CSS, liquid glass).
  // Gated on webServer internally, so headless profiles mount this plugin unchanged.
  if (config.enableBrandAssets !== false) {
    installBrandAssets(ctx as unknown as { inject: never }, {
      liquidGlass: config.enableLiquidGlass !== false,
      workspacePicker: config.workspacePicker !== false,
    })
  }

  if (config.enableCommands !== false) {
    ctx.inject(['commands'], (scope) => {
      registerProteusCommands((scope as unknown as { commands: CommandsSeam }).commands, {
        config: toolsConfig,
        shell,
        cwd,
      })
    })
  }

  if (config.enableSkill !== false) {
    ctx.inject(['skills'], (scope) => {
      registerProteusSkill((scope as unknown as { skills: SkillsSeam }).skills)
    })
  }
}
