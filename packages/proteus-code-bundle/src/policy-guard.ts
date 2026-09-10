/**
 * Wires the execution policy into the host's tool dispatch.
 *
 * The `tools/pre-execute` event is a waterfall: listeners delegate with
 * `next()`. This guard only ever TIGHTENS — a `forbidden` verdict denies, a
 * `prompt` verdict asks, and anything else (including an explicit `allow`)
 * delegates so the host's own sandbox and approval listeners still run.
 *
 * Policy resolution lives in {@link PolicyRuntime} so this guard and the
 * introspection tool always agree on the answer.
 *
 * @module @proteus-code/dsh-proteus-code/policy-guard
 */

import type { PolicyRuntime } from './policy-runtime.ts'
import type { Verdict } from './policy.ts'

/** Tool names whose `command` argument is a shell command line. */
const SHELL_TOOLS = new Set(['bash', 'pwsh'])

/** Structural view of the pending call passed to `tools/pre-execute`. */
export interface PendingToolCall {
  readonly name: string
  readonly arguments: unknown
}

/** Structural view of the host tool runtime, for `ctx.on` registration. */
export interface HostContext {
  on(
    event: 'tools/pre-execute',
    listener: (exec: PendingToolCall, next: () => Promise<unknown>) => Promise<unknown>,
  ): unknown
}

/**
 * Build the command line a policy should judge for one pending call.
 *
 * Shell tools are judged on their `command`. The `proteus_cli` escape hatch is
 * judged as the `proteus <subcommand> ...` line it will run, so a single rule
 * covers both a raw shell invocation and this tool. Other tools return
 * `undefined` and are left to the host's own policy.
 */
export function commandForCall(call: PendingToolCall): string | undefined {
  const args = (call.arguments ?? {}) as Record<string, unknown>
  if (SHELL_TOOLS.has(call.name)) {
    return typeof args.command === 'string' ? args.command : undefined
  }
  if (call.name === 'proteus_cli') {
    const subcommand = typeof args.subcommand === 'string' ? args.subcommand : ''
    if (!subcommand) return undefined
    const rest = Array.isArray(args.args) ? args.args.map((value) => String(value)) : []
    return ['proteus', subcommand, ...rest].join(' ')
  }
  return undefined
}

/** Render a verdict into the sentence a human sees in the prompt or denial. */
export function explainVerdict(verdict: Verdict, command: string): string {
  const rule = verdict.matched[0]?.id
  const rationale = verdict.justification ?? 'no justification was recorded for this rule'
  const label = verdict.decision === 'forbidden' ? 'Blocked' : 'Confirmation required'
  const suffix = rule ? ` (rule: ${rule})` : ''
  return `${label} by proteus-code policy${suffix}. ${rationale}\nCommand: ${command}`
}

/**
 * Install the policy guard on the pre-execute waterfall.
 *
 * A policy file that exists but cannot be parsed makes the guard fail CLOSED —
 * the call is denied with the parse error as the reason — rather than silently
 * running unguarded.
 */
export function installPolicyGuard(
  ctx: HostContext,
  options: { runtime: PolicyRuntime },
): void {
  const { runtime } = options

  ctx.on('tools/pre-execute', async (exec, next) => {
    const command = commandForCall(exec)
    if (command === undefined) return await next()

    const { verdict, resolved } = await runtime.evaluate(command)

    if (resolved.error) {
      // Fail closed, and say exactly why so the operator can fix it.
      return {
        kind: 'deny',
        reason: `proteus-code policy failed to load, so this call was blocked.\n${resolved.error}`,
      }
    }
    if (verdict.decision === 'forbidden') {
      return { kind: 'deny', reason: explainVerdict(verdict, command) }
    }
    if (verdict.decision === 'prompt') {
      return { kind: 'ask', reason: explainVerdict(verdict, command) }
    }
    // `allow` and no-match both delegate, so the host's own policy still runs.
    return await next()
  })
}
