/**
 * Execution policy engine, adapted from OpenAI Codex's `codex-execpolicy`.
 *
 * Codex's design has three properties worth keeping, and this module keeps all
 * three:
 *
 *   1. **Prefix rules.** A rule is an ordered token pattern matched against the
 *      start of a command. Any token may list alternatives, so one rule covers
 *      `rm -r` and `rm -rf` without duplication.
 *   2. **Self-testing rules.** A rule may carry `match` / `notMatch` examples
 *      that are validated when the policy loads, so a typo fails loudly at
 *      startup instead of silently allowing the wrong thing.
 *   3. **Host-executable pinning.** A basename rule for `git` matches only the
 *      absolute paths declared for `git`, so a same-named binary dropped
 *      elsewhere (e.g. `/tmp/evil/git`) cannot inherit an allow rule.
 *
 * Two adaptations to this host, both deliberate:
 *
 *   - Rules are plain JSON/YAML data rather than Starlark. The host already
 *     parses YAML for plugin config, so a second language runtime would be
 *     cost without benefit; the semantics above are what matter.
 *   - Decisions map onto DeepSeek Harness's `tools/pre-execute` waterfall. This
 *     engine can only ever TIGHTEN the host's own sandbox and approval policy:
 *     `allow` delegates to the rest of the chain via `next()` rather than
 *     short-circuiting, so DSH's own permission presets still apply. Loosening
 *     policy stays the host's decision, not a plugin's.
 *
 * @module @proteus-code/dsh-proteus-code/policy
 */

import { readFileSync, statSync } from 'node:fs'
import path from 'node:path'

/** What the policy says about a command. Ordered least → most restrictive. */
export type Decision = 'allow' | 'prompt' | 'forbidden'

/** One element of a prefix pattern: a literal token, or alternatives. */
export type PatternToken = string | readonly string[]

/** A validated prefix rule. */
export interface PolicyRule {
  /** Stable id for logs and diagnostics; synthesized from the pattern when absent. */
  readonly id: string
  /** Ordered tokens. The first entry must be a single string, never alternatives. */
  readonly pattern: readonly PatternToken[]
  readonly decision: Decision
  /** Human-readable rationale, surfaced in approval prompts and denials. */
  readonly justification?: string
  /** Example invocations that MUST match — validated at load time. */
  readonly match?: readonly (readonly string[])[]
  /** Example invocations that MUST NOT match — validated at load time. */
  readonly notMatch?: readonly (readonly string[])[]
}

/** Constrains which absolute paths a basename rule may resolve through. */
export interface HostExecutable {
  readonly name: string
  readonly paths: readonly string[]
}

/** A complete, validated policy. */
export interface Policy {
  readonly rules: readonly PolicyRule[]
  readonly hostExecutables: readonly HostExecutable[]
}

/** Outcome of evaluating one command against a policy. */
export interface Verdict {
  readonly decision: Decision
  /** The rules that matched, most restrictive first. */
  readonly matched: readonly PolicyRule[]
  /** Justification from the winning rule, when it has one. */
  readonly justification?: string
}

/** Rule order that decides which match wins: the most restrictive. */
const RESTRICTIVENESS: Record<Decision, number> = { allow: 0, prompt: 1, forbidden: 2 }

/** Raised for a malformed policy; message names the offending rule. */
export class PolicyError extends Error {
  constructor(message: string) {
    super(`proteus-code policy: ${message}`)
    this.name = 'PolicyError'
  }
}

/**
 * Split a command into tokens, honoring single and double quotes and
 * backslash escapes the way a POSIX shell does for the prefix we compare.
 *
 * This is intentionally a tokenizer, not an evaluator: the policy never runs
 * the command, it only compares the leading tokens.
 */
export function tokenizeCommand(command: string): string[] {
  const tokens: string[] = []
  let current = ''
  let started = false
  let quote: "'" | '"' | undefined

  for (let i = 0; i < command.length; i += 1) {
    const char = command[i] as string

    if (quote === "'") {
      if (char === "'") quote = undefined
      else current += char
      continue
    }

    if (quote === '"') {
      if (char === '"') {
        quote = undefined
      } else if (char === '\\' && i + 1 < command.length) {
        i += 1
        current += command[i] as string
      } else {
        current += char
      }
      continue
    }

    if (char === "'" || char === '"') {
      quote = char
      started = true
    } else if (char === '\\' && i + 1 < command.length) {
      i += 1
      current += command[i] as string
      started = true
    } else if (char === ' ' || char === '\t' || char === '\n' || char === '\r') {
      if (started || current.length > 0) {
        tokens.push(current)
        current = ''
        started = false
      }
    } else {
      current += char
      started = true
    }
  }
  if (started || current.length > 0) tokens.push(current)
  return tokens
}

/** Does a rule's pattern match the start of `tokens`? */
export function matchesPrefix(rule: PolicyRule, tokens: readonly string[]): boolean {
  if (tokens.length < rule.pattern.length) return false
  // The first token is positional so the engine can index rules by it.
  const first = rule.pattern[0]
  if (typeof first !== 'string' || tokens[0] !== first) return false
  for (let i = 1; i < rule.pattern.length; i += 1) {
    const expected = rule.pattern[i] as PatternToken
    const actual = tokens[i] as string
    if (typeof expected === 'string') {
      if (expected !== actual) return false
    } else if (!expected.includes(actual)) {
      return false
    }
  }
  return true
}

/** Synthesize a readable id for a rule that did not declare one. */
function ruleId(rule: { id?: string; pattern: readonly PatternToken[] }): string {
  if (rule.id && rule.id.trim()) return rule.id
  return rule.pattern
    .map((token) => (typeof token === 'string' ? token : `[${token.join('|')}]`))
    .join(' ')
}

/** Read one pattern token, rejecting empty alternatives and non-strings. */
function readPatternToken(value: unknown, where: string): PatternToken {
  if (typeof value === 'string') {
    if (value.length === 0) throw new PolicyError(`${where}: pattern tokens must be non-empty`)
    return value
  }
  if (Array.isArray(value)) {
    if (value.length === 0) throw new PolicyError(`${where}: an alternatives list must not be empty`)
    return value.map((alt) => {
      if (typeof alt !== 'string' || alt.length === 0) {
        throw new PolicyError(`${where}: alternatives must be non-empty strings`)
      }
      return alt
    })
  }
  throw new PolicyError(`${where}: a pattern token must be a string or a list of strings`)
}

/** Read an example list (`match` / `notMatch`) into token vectors. */
function readExamples(value: unknown, where: string): readonly string[][] {
  if (!Array.isArray(value)) throw new PolicyError(`${where}: expected a list of examples`)
  return value.map((example) => {
    if (typeof example === 'string') return tokenizeCommand(example)
    if (Array.isArray(example) && example.every((t) => typeof t === 'string')) {
      return example as string[]
    }
    throw new PolicyError(`${where}: each example must be a string or a list of strings`)
  })
}

function readDecision(value: unknown, where: string): Decision {
  if (value === undefined) return 'allow'
  if (value === 'allow' || value === 'prompt' || value === 'forbidden') return value
  throw new PolicyError(
    `${where}: decision must be "allow", "prompt", or "forbidden" (got ${JSON.stringify(value)})`,
  )
}

/**
 * Validate raw policy data into a {@link Policy}.
 *
 * Every `match` example must match its rule and every `notMatch` example must
 * not, so an incorrect rule is a load-time failure. Throws {@link PolicyError}
 * with the offending rule named.
 */
export function parsePolicy(source: unknown): Policy {
  if (source === undefined || source === null) return { rules: [], hostExecutables: [] }
  if (typeof source !== 'object' || Array.isArray(source)) {
    throw new PolicyError('the policy must be an object with `rules` and/or `hostExecutables`')
  }
  const raw = source as { rules?: unknown; hostExecutables?: unknown }

  const rawRules = raw.rules ?? []
  if (!Array.isArray(rawRules)) throw new PolicyError('`rules` must be a list')

  const rules: PolicyRule[] = rawRules.map((entry, index) => {
    if (typeof entry !== 'object' || entry === null || Array.isArray(entry)) {
      throw new PolicyError(`rule #${index + 1}: expected an object`)
    }
    const candidate = entry as Record<string, unknown>
    const where = `rule #${index + 1}`

    const pattern = candidate.pattern
    if (!Array.isArray(pattern) || pattern.length === 0) {
      throw new PolicyError(`${where}: \`pattern\` must be a non-empty list of tokens`)
    }
    const tokens = pattern.map((token) => readPatternToken(token, where))
    // Keying and exact-match ordering rely on a fixed first token.
    if (typeof tokens[0] !== 'string') {
      throw new PolicyError(`${where}: the first \`pattern\` token must be a single string`)
    }

    const rule: PolicyRule = {
      id: ruleId({ id: typeof candidate.id === 'string' ? candidate.id : undefined, pattern: tokens }),
      pattern: tokens,
      decision: readDecision(candidate.decision, where),
      ...(typeof candidate.justification === 'string' ? { justification: candidate.justification } : {}),
      ...(candidate.match !== undefined ? { match: readExamples(candidate.match, `${where}.match`) } : {}),
      ...(candidate.notMatch !== undefined
        ? { notMatch: readExamples(candidate.notMatch, `${where}.notMatch`) }
        : {}),
    }

    // Self-tests: the rule proves itself at load time.
    for (const example of rule.match ?? []) {
      if (!matchesPrefix(rule, example)) {
        throw new PolicyError(
          `${where} (${rule.id}): \`match\` example [${example.join(' ')}] does not match the pattern`,
        )
      }
    }
    for (const example of rule.notMatch ?? []) {
      if (matchesPrefix(rule, example)) {
        throw new PolicyError(
          `${where} (${rule.id}): \`notMatch\` example [${example.join(' ')}] unexpectedly matches`,
        )
      }
    }
    return rule
  })

  const rawHosts = raw.hostExecutables ?? []
  if (!Array.isArray(rawHosts)) throw new PolicyError('`hostExecutables` must be a list')
  const hostExecutables: HostExecutable[] = rawHosts.map((entry, index) => {
    const where = `hostExecutables #${index + 1}`
    if (typeof entry !== 'object' || entry === null) throw new PolicyError(`${where}: expected an object`)
    const candidate = entry as Record<string, unknown>
    if (typeof candidate.name !== 'string' || candidate.name.length === 0) {
      throw new PolicyError(`${where}: \`name\` must be a non-empty string`)
    }
    if (!Array.isArray(candidate.paths) || candidate.paths.length === 0) {
      throw new PolicyError(`${where}: \`paths\` must be a non-empty list`)
    }
    const paths = candidate.paths.map((value) => {
      if (typeof value !== 'string' || !path.isAbsolute(value)) {
        throw new PolicyError(`${where}: every path must be an absolute string (got ${JSON.stringify(value)})`)
      }
      return value
    })
    return { name: candidate.name, paths }
  })

  return { rules, hostExecutables }
}

/** How to resolve an absolute program path before matching. */
export interface EvaluateOptions {
  /**
   * Whether an unmatched absolute executable may fall back to basename rules.
   * Mirrors Codex's `--resolve-host-executables`.
   */
  readonly resolveHostExecutables?: boolean
}

/**
 * Evaluate one command against a policy.
 *
 * Precedence follows Codex: exact first-token rules are tried first, and only
 * when none match may an absolute program path fall back to basename rules —
 * and then only for paths declared in `hostExecutables`. Among the rules that
 * do match, the most restrictive decision wins.
 */
export function evaluatePolicy(
  policy: Policy,
  command: string,
  options: EvaluateOptions = {},
): Verdict {
  const tokens = tokenizeCommand(command)
  if (tokens.length === 0) return { decision: 'allow', matched: [] }

  // Exact rules first: the first token must equal the command's program token.
  let matched = policy.rules.filter((rule) => matchesPrefix(rule, tokens))

  if (matched.length === 0 && options.resolveHostExecutables) {
    const program = tokens[0] as string
    if (path.isAbsolute(program)) {
      const basename = path.basename(program)
      const pinned = policy.hostExecutables.find((entry) => entry.name === basename)
      // A declared host executable constrains basename fallback to its paths;
      // an undeclared basename may fall back freely.
      if (!pinned || pinned.paths.includes(program)) {
        matched = policy.rules.filter(
          (rule) => rule.pattern[0] === basename && matchesPrefix(rule, [basename, ...tokens.slice(1)]),
        )
      }
    }
  }

  if (matched.length === 0) return { decision: 'allow', matched: [] }

  const ordered = [...matched].sort(
    (a, b) => RESTRICTIVENESS[b.decision] - RESTRICTIVENESS[a.decision],
  )
  const winner = ordered[0] as PolicyRule
  return {
    decision: winner.decision,
    matched: ordered,
    ...(winner.justification ? { justification: winner.justification } : {}),
  }
}

/** File names searched for a workspace policy, in priority order. */
export const POLICY_FILE_NAMES = ['.proteus-code/policy.json', 'proteus-code.policy.json'] as const

/** Walk upward from `startDir` looking for a workspace policy file. */
export function discoverPolicyPath(startDir: string): string | undefined {
  let current = path.resolve(startDir)
  for (;;) {
    for (const name of POLICY_FILE_NAMES) {
      const candidate = path.join(current, name)
      try {
        if (statSync(candidate).isFile()) return candidate
      } catch {
        // Not present; keep walking.
      }
    }
    const parent = path.dirname(current)
    if (parent === current) return undefined
    current = parent
  }
}

/**
 * Parse a policy file's contents.
 *
 * JSON is supported natively. YAML is accepted only when the running harness
 * happens to expose a parser on the module graph (`js-yaml`, which DSH itself
 * depends on); this plugin declares no dependency on one, so a YAML file without
 * a parser fails with an actionable message rather than a resolution stack.
 */
export async function parsePolicyFile(filePath: string): Promise<Policy> {
  const text = readFileSync(filePath, 'utf8')
  if (filePath.endsWith('.json')) return parsePolicy(JSON.parse(text))

  let load: ((source: string) => unknown) | undefined
  try {
    const moduleName = 'js-yaml'
    const yaml = (await import(/* @vite-ignore */ moduleName)) as {
      load?: (source: string) => unknown
      default?: { load?: (source: string) => unknown }
    }
    load = yaml.load ?? yaml.default?.load
  } catch {
    load = undefined
  }
  if (!load) {
    throw new PolicyError(
      `no YAML parser is available, so ${filePath} cannot be read. ` +
        `Rename it to .json, or put the rules in the plugin's \`policy\` config.`,
    )
  }
  try {
    return parsePolicy(load(text))
  } catch (error) {
    if (error instanceof PolicyError) throw error
    throw new PolicyError(`could not parse ${filePath} as YAML: ${(error as Error).message}`)
  }
}
