/**
 * The Proteus tool surface.
 *
 * Every tool is a thin, typed wrapper over one real `proteus` subcommand; the
 * shared factory turns a canonical {@link CliResult} into both the model-facing
 * text and the structured value the harness stores. No tool invents a command
 * that the CLI does not implement.
 *
 * @module @proteus-code/dsh-proteus-code/tools
 */

import { defineTool } from '@deepseek-ai/dsh-tools'
import type { ParameterPropertySpec, ParameterSchemaSpec } from '@deepseek-ai/dsh-tools'
import {
  createCliRunner,
  resolveInvocation,
  toCommandString,
  type CliResult,
  type ProteusInvocation,
  type ShellSeam,
} from './proteus-cli.ts'

/** Plugin options the tools read. */
export interface ToolsConfig {
  proteusRoot?: string | undefined
  proteusCommand?: string | undefined
  timeoutMs?: number | undefined
}

/** Canonical JSON value every Proteus tool returns. */
interface CliValue {
  ok: boolean
  exitCode: number | null
  command: string
  cwd: string
  stdout: string
  stderr: string
  truncated: boolean
  durationMs: number
}

const CLI_OUTPUT_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    ok: { type: 'boolean', description: 'Whether the command exited zero.' },
    exitCode: {
      oneOf: [{ type: 'integer' }, { type: 'null' }],
      description: 'Process exit code, or null when terminated by a signal.',
    },
    command: { type: 'string', description: 'The exact command line that ran.' },
    cwd: { type: 'string', description: 'Working directory of the run.' },
    stdout: { type: 'string', description: 'Captured standard output (tail when truncated).' },
    stderr: { type: 'string', description: 'Captured standard error (tail when truncated).' },
    truncated: { type: 'boolean', description: 'Whether collected output was truncated.' },
    durationMs: { type: 'integer', description: 'Wall-clock duration in milliseconds.' },
  },
} as const

/** Build the model-facing content from one canonical CLI value. */
function renderCli(_args: unknown, value: unknown): { type: 'text'; text: string }[] {
  const v = value as CliValue
  const parts: string[] = []
  parts.push(v.ok ? `✅ ${v.command}` : `❌ ${v.command} (exit ${v.exitCode ?? 'signal'})`)
  if (v.stdout.trim()) parts.push(v.stdout.trimEnd())
  if (v.stderr.trim()) parts.push(`stderr:\n${v.stderr.trimEnd()}`)
  if (v.truncated) parts.push('[output truncated; see the session spill file for the full stream]')
  return [{ type: 'text', text: parts.join('\n\n') }]
}

/** Turn one CLI result into the canonical value. */
function toValue(result: CliResult): CliValue {
  return {
    ok: result.ok,
    exitCode: result.exitCode,
    command: result.command,
    cwd: result.cwd,
    stdout: result.stdout,
    stderr: result.stderr,
    truncated: result.truncated,
    durationMs: result.durationMs,
  }
}

/** Definition handed to the shared factory. */
interface CliToolSpec {
  name: string
  description: string
  parameters: ParameterSchemaSpec
  /** Build the Proteus subcommand argv from validated arguments. */
  subcommand: (args: Record<string, unknown>) => string[]
}

/**
 * Build the shared tool surface. `cwd` is read per call so a session that
 * changes its working directory re-resolves the Proteus checkout.
 */
export function createProteusTools(options: {
  config: ToolsConfig
  shell?: ShellSeam | undefined
  cwd: () => string
}) {
  const runner = createCliRunner(options.shell)

  async function exec(
    spec: CliToolSpec,
    rawArgs: unknown,
    signal: AbortSignal | undefined,
  ): Promise<CliValue> {
    const args = (rawArgs ?? {}) as Record<string, unknown>
    let invocation: ProteusInvocation
    try {
      invocation = resolveInvocation(options.config, options.cwd())
    } catch (error) {
      return toValue(failure((error as Error).message))
    }
    const argv = [...invocation.argvPrefix, ...spec.subcommand(args)]
    const command = toCommandString(argv)
    try {
      const result = await runner.run({
        command,
        workdir: invocation.root,
        timeoutMs: options.config.timeoutMs,
        signal,
      })
      return toValue(result)
    } catch (error) {
      return toValue(failure(`${command}: ${(error as Error).message}`))
    }
  }

  const build = (spec: CliToolSpec) =>
    defineTool({
      name: spec.name,
      description: spec.description,
      parameters: spec.parameters,
      output: {
        schema: CLI_OUTPUT_SCHEMA,
        render: renderCli,
      },
      async execute(args, execCtx) {
        return await exec(spec, args, execCtx.signal)
      },
    })

  const str = (description: string, required = false): ParameterPropertySpec => ({
    type: 'string',
    description,
    ...(required ? { required: true as const } : {}),
  })
  const bool = (description: string): ParameterPropertySpec => ({ type: 'boolean', description })

  return [
    build({
      name: 'proteus_health',
      description:
        'Run `proteus health` on a Proteus project: reports configuration, package, and toolchain health with actionable findings. Start here when diagnosing a Proteus workspace.',
      parameters: {
        dir: str('Project directory to inspect. Defaults to the session working directory.'),
      },
      subcommand: (args) => ['health', ...opt(args.dir)],
    }),
    build({
      name: 'proteus_check',
      description:
        'Run the `proteus check` gate suite (styles, CSS, router, CLI conformance). Use before declaring work done; a nonzero exit means a gate failed.',
      parameters: {
        dir: str('Project directory to check. Defaults to the session working directory.'),
        strictCss: bool('Set false to disable the strict CSS gate (--no-strict-css).'),
        strictStyle: bool('Set false to disable the strict style gate (--no-strict-style).'),
        strictRouter: bool('Set false to disable the strict router gate (--no-strict-router).'),
        strictCli: bool('Set false to disable the strict CLI gate (--no-strict-cli).'),
      },
      subcommand: (args) => [
        'check',
        ...opt(args.dir),
        ...off('--no-strict-css', args.strictCss),
        ...off('--no-strict-style', args.strictStyle),
        ...off('--no-strict-router', args.strictRouter),
        ...off('--no-strict-cli', args.strictCli),
      ],
    }),
    build({
      name: 'proteus_build',
      description:
        'Run `proteus build` for a target. Use --target web for the web bundle, skyline for the mini-program Skyline build, or all for both.',
      parameters: {
        dir: str('Project directory to build. Defaults to the session working directory.'),
        target: {
          type: 'string',
          enum: ['web', 'skyline', 'all'],
          description: 'Build target. Required for the programmatic build path.',
          required: true,
        },
        out: str('Output directory override (--out).'),
        debug: bool('Emit the per-file decision trace under .transform-debug/ (--debug).'),
        compiler: {
          type: 'string',
          enum: ['node', 'rust'],
          description: 'Compiler backend; rust also runs the Node/Rust equivalence check.',
        },
        px2rpx: bool('Set false to disable compile-time px→rpx conversion (--no-px2rpx).'),
      },
      subcommand: (args) => [
        'build',
        typeof args.dir === 'string' ? args.dir : '.',
        '--target',
        String(args.target ?? 'web'),
        ...val('--out', args.out),
        ...on('--debug', args.debug),
        ...val('--compiler', args.compiler),
        ...off('--no-px2rpx', args.px2rpx),
      ],
    }),
    build({
      name: 'proteus_explain',
      description:
        'Run `proteus explain`. On a .vue file it prints the decision trace (which transform rules that file actually triggered); on a rule id it prints the rule AI manual (what / why / when / example / verify).',
      parameters: {
        target: str('A .vue file path or a rule id such as `tag/div-to-view`.', true),
      },
      subcommand: (args) => ['explain', String(args.target ?? '')],
    }),
    build({
      name: 'proteus_rules',
      description:
        'Run `proteus rules` to list the compiler rule catalog (the AI-manual directory) grouped by phase.',
      parameters: {
        phase: {
          type: 'string',
          enum: ['template', 'script', 'style', 'validate'],
          description: 'Restrict the listing to one compilation phase.',
        },
      },
      subcommand: (args) => ['rules', ...opt(args.phase)],
    }),
    build({
      name: 'proteus_audit',
      description:
        'Run `proteus audit` for one audit kind: module (module boundaries), d2 (design-system conformance), all (every audit), coverage (capability coverage), or devtools-budget.',
      parameters: {
        kind: {
          type: 'string',
          enum: ['module', 'd2', 'all', 'coverage', 'devtools-budget'],
          description: 'Which audit to run.',
          required: true,
        },
        dir: str('Project directory for audits that take one. Defaults to the session working directory.'),
        dist: str('Built output directory (--dist) for the module audit.'),
      },
      subcommand: (args) => [
        'audit',
        String(args.kind ?? 'all'),
        ...opt(args.dir),
        ...val('--dist', args.dist),
      ],
    }),
    build({
      name: 'proteus_conformance',
      description:
        'Run `proteus conformance` SPI conformance suites. Use --repo to scan a host repository for forbidden forks (G-42 governance), or --backend to test an external compiler backend.',
      parameters: {
        backend: str('External backend module path, optionally #namedExport.'),
        only: str('Run a single suite by group id, e.g. C-03.'),
        repo: str('Host repository root to scan for forbidden forks (--repo).'),
        demo: bool('Run the conformance demo instead of a backend suite.'),
      },
      subcommand: (args) => [
        'conformance',
        ...val('--backend', args.backend),
        ...val('--only', args.only),
        ...val('--repo', args.repo),
        ...on('--demo', args.demo),
      ],
    }),
    build({
      name: 'proteus_migrate_mp',
      description:
        'Run the mini-program migration codemod (`proteus migrate mp`) on a file or directory: rewrites automatic tags and synchronous storage, and marks callback-style APIs as manual. Idempotent; use dryRun to preview without writing.',
      parameters: {
        target: str('File or directory to migrate.', true),
        dryRun: bool('Report planned changes without writing (--dry-run).'),
      },
      subcommand: (args) => ['migrate', 'mp', String(args.target ?? ''), ...on('--dry-run', args.dryRun)],
    }),
    build({
      name: 'proteus_cli',
      description:
        'Escape hatch: run `proteus <subcommand> [args...]` directly for commands without a dedicated tool. The subcommand must be one of the known Proteus subcommands.',
      parameters: {
        subcommand: str('Proteus subcommand, e.g. `health` or `css:check`.', true),
        args: {
          type: 'array',
          items: { type: 'string', description: 'One argument.' },
          description: 'Arguments passed through verbatim, in order.',
        },
      },
      subcommand: (args) => {
        const sub = String(args.subcommand ?? '')
        if (!ALLOWED_SUBCOMMANDS.has(sub)) {
          throw new Error(
            `proteus_cli: unknown subcommand "${sub}". Allowed: ${[...ALLOWED_SUBCOMMANDS].sort().join(', ')}`,
          )
        }
        const rest = Array.isArray(args.args) ? args.args.map((a) => String(a)) : []
        return [sub, ...rest]
      },
    }),
  ]
}

/** Subcommands the escape hatch accepts, mirroring the CLI's command table. */
export const ALLOWED_SUBCOMMANDS = new Set([
  'build',
  'dev',
  'check',
  'gate',
  'conformance',
  'health',
  'css:check',
  'style:check',
  'config:check',
  'app-config:check',
  'i18n:check',
  'router:check',
  'module:check',
  'module:duplicates',
  'audit',
  'capabilities:manifest',
  'capabilities:check',
  'api-check',
  'components:audit',
  'fluid:check',
  'explain',
  'rules',
  'generate',
  'migrate',
  'ci:init',
  'gen',
  'host',
  'test',
  'init',
  'version',
  'help',
])

/** Synthesize a failed result for an error raised before the CLI could run. */
function failure(message: string): CliResult {
  return {
    ok: false,
    exitCode: null,
    signal: null,
    command: message,
    cwd: '',
    stdout: '',
    stderr: message,
    truncated: false,
    durationMs: 0,
  }
}

/** Include a string argument when present and non-empty. */
function opt(value: unknown): string[] {
  return typeof value === 'string' && value.trim() ? [value] : []
}

/** Include `--flag value` when present and non-empty. */
function val(flag: string, value: unknown): string[] {
  return typeof value === 'string' && value.trim() ? [flag, value] : []
}

/** Include a boolean flag when true. */
function on(flag: string, value: unknown): string[] {
  return value === true ? [flag] : []
}

/** Include a negated flag when explicitly false. */
function off(flag: string, value: unknown): string[] {
  return value === false ? [flag] : []
}
