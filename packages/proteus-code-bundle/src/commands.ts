/**
 * Human slash commands.
 *
 * These run the Proteus CLI directly against the project without creating a
 * model message, mirroring the capability DSH's own `/compact`-style commands
 * use. Registration is effect-based, so unloading the plugin removes them.
 *
 * @module @proteus-code/dsh-proteus-code/commands
 */

import {
  createCliRunner,
  resolveInvocation,
  toCommandString,
  type CliResult,
  type ShellSeam,
} from './proteus-cli.ts'
import type { ToolsConfig } from './tools.ts'

/** Structural view of `ctx.commands` (`@deepseek-ai/dsh-commands`). */
export interface CommandsSeam {
  register(definition: {
    name: string
    description: string
    input?: { hint: string }
    handler: (invocation: {
      rawInput: string
      signal: AbortSignal
    }) => CommandOutcome | Promise<CommandOutcome>
  }): () => void
}

/** Normalized outcome a command handler returns. */
export interface CommandOutcome {
  kind: 'success' | 'error'
  text?: string
}

/** One command definition driven by a fixed Proteus subcommand. */
interface CommandSpec {
  name: string
  description: string
  hint: string
  /** Build the Proteus argv from the command's raw input. */
  argv: (rawInput: string) => string[]
}

/** Format a CLI result as the short text a command result carries. */
function summarize(result: CliResult): CommandOutcome {
  const body = [result.stdout.trimEnd(), result.stderr.trimEnd()].filter(Boolean).join('\n')
  const head = result.ok
    ? `proteus: ok (${result.durationMs}ms)`
    : `proteus: failed (exit ${result.exitCode ?? 'signal'})`
  return { kind: result.ok ? 'success' : 'error', text: body ? `${head}\n${body}` : head }
}

/** Register the Proteus command surface. */
export function registerProteusCommands(
  commands: CommandsSeam,
  options: { config: ToolsConfig; shell?: ShellSeam | undefined; cwd: () => string },
): void {
  const runner = createCliRunner(options.shell)

  const specs: CommandSpec[] = [
    {
      name: 'proteus-health',
      description: 'Run `proteus health` on the current project',
      hint: '[dir]',
      argv: (input) => ['health', ...tokens(input)],
    },
    {
      name: 'proteus-check',
      description: 'Run the `proteus check` gate suite',
      hint: '[dir]',
      argv: (input) => ['check', ...tokens(input)],
    },
    {
      name: 'proteus-build',
      description: 'Build a Proteus target',
      hint: '<web|skyline|all> [dir]',
      argv: (input) => {
        const [target = 'web', ...rest] = tokens(input)
        return ['build', ...(rest.length ? rest : ['.']), '--target', target]
      },
    },
    {
      name: 'proteus-rules',
      description: 'List the compiler rule catalog',
      hint: '[template|script|style|validate]',
      argv: (input) => ['rules', ...tokens(input)],
    },
    {
      name: 'proteus-explain',
      description: 'Explain a .vue file decision trace or a rule id',
      hint: '<file|rule-id>',
      argv: (input) => {
        const [target = ''] = tokens(input)
        return target ? ['explain', target] : ['rules']
      },
    },
  ]

  for (const spec of specs) {
    commands.register({
      name: spec.name,
      description: spec.description,
      input: { hint: spec.hint },
      handler: async ({ rawInput, signal }) => {
        try {
          const invocation = resolveInvocation(options.config, options.cwd())
          const command = toCommandString([...invocation.argvPrefix, ...spec.argv(rawInput)])
          const result = await runner.run({
            command,
            workdir: invocation.root,
            timeoutMs: options.config.timeoutMs,
            signal,
          })
          return summarize(result)
        } catch (error) {
          return { kind: 'error', text: (error as Error).message }
        }
      },
    })
  }
}

/** Split command input into whitespace-separated tokens. */
function tokens(input: string): string[] {
  return input.trim().split(/\s+/).filter(Boolean)
}
