/**
 * The policy introspection tool.
 *
 * Mirrors `codex execpolicy check`: it answers "what would the policy decide for
 * this command, and which rule decides it?" WITHOUT running anything. That lets
 * the model check itself before attempting a command, and lets a human audit why
 * a call was blocked.
 *
 * @module @proteus-code/dsh-proteus-code/policy-tool
 */

import { defineTool } from '@deepseek-ai/dsh-tools'
import type { PolicyRuntime } from './policy-runtime.ts'

/** Canonical value returned by the policy tool. */
interface PolicyValue {
  decision: 'allow' | 'prompt' | 'forbidden'
  /** Rule ids that matched, most restrictive first. */
  matchedRules: string[]
  justification: string | null
  /** The policy file that contributed rules, when one applied. */
  policyFile: string | null
  /** Number of rules in effect, so a caller can tell "no match" from "no rules". */
  ruleCount: number
}

const POLICY_OUTPUT_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    decision: {
      type: 'string',
      enum: ['allow', 'prompt', 'forbidden'],
      description:
        'allow = the policy does not restrict it (the host sandbox and approval policy still apply); prompt = ask the user; forbidden = blocked.',
    },
    matchedRules: {
      type: 'array',
      items: { type: 'string', description: 'A matched rule id.' },
      description: 'Matched rule ids, most restrictive first.',
    },
    justification: {
      oneOf: [{ type: 'string' }, { type: 'null' }],
      description: 'Why the winning rule exists, when recorded.',
    },
    policyFile: {
      oneOf: [{ type: 'string' }, { type: 'null' }],
      description: 'The policy file that contributed rules, when one applied.',
    },
    ruleCount: { type: 'integer', description: 'How many rules are in effect.' },
  },
} as const

function renderPolicy(_args: unknown, value: unknown): { type: 'text'; text: string }[] {
  const v = value as PolicyValue
  const lines = [`decision: ${v.decision}`, `rules in effect: ${v.ruleCount}`]
  if (v.matchedRules.length > 0) lines.push(`matched: ${v.matchedRules.join(', ')}`)
  else lines.push('matched: none')
  if (v.justification) lines.push(`justification: ${v.justification}`)
  if (v.policyFile) lines.push(`policy file: ${v.policyFile}`)
  if (v.decision === 'allow' && v.matchedRules.length === 0) {
    lines.push('(the host sandbox and approval policy still apply)')
  }
  return [{ type: 'text', text: lines.join('\n') }]
}

/**
 * Build the policy tool. It never executes the command; it only evaluates the
 * same runtime the guard uses, so its answer matches what dispatch will do.
 */
export function createPolicyTool(runtime: PolicyRuntime) {
  return defineTool({
    name: 'proteus_policy',
    description:
      'Check what the workspace execution policy decides for a shell command WITHOUT running it. ' +
      'Returns allow / prompt / forbidden plus the rule that decided it. Use this before attempting ' +
      'a command you are unsure about, or to explain why a command was blocked.',
    parameters: {
      command: {
        type: 'string',
        required: true,
        description: 'The shell command line to evaluate, e.g. `git push origin main`.',
      },
    },
    output: {
      schema: POLICY_OUTPUT_SCHEMA,
      render: renderPolicy,
    },
    async execute(args) {
      const command = String((args as { command?: unknown }).command ?? '')
      const { verdict, resolved } = await runtime.evaluate(command)
      return {
        decision: verdict.decision,
        matchedRules: verdict.matched.map((rule) => rule.id),
        justification: verdict.justification ?? null,
        policyFile: resolved.filePath ?? null,
        ruleCount: resolved.policy.rules.length,
      } satisfies PolicyValue
    },
  })
}
