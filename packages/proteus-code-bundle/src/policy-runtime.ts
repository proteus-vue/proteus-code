/**
 * Policy runtime: one owner of policy resolution, shared by the guard and the
 * introspection tool.
 *
 * Keeping resolution in one place matters because the guard and the tool must
 * agree. If they resolved independently, a tool could report "allowed" while
 * the guard denied the call.
 *
 * @module @proteus-code/dsh-proteus-code/policy-runtime
 */

import { statSync } from 'node:fs'
import {
  discoverPolicyPath,
  evaluatePolicy,
  parsePolicy,
  parsePolicyFile,
  PolicyError,
  type EvaluateOptions,
  type Policy,
  type Verdict,
} from './policy.ts'

/** Config the runtime reads. */
export interface PolicyRuntimeConfig {
  /** Inline rules from plugin config; validated when the runtime is created. */
  policy?: unknown
  /** Explicit policy file path; disables discovery when set. */
  policyFile?: string
  /** Whether to search for a workspace policy file. */
  discoverPolicy?: boolean
  /** Whether absolute program paths may fall back to basename rules. */
  resolveHostExecutables?: boolean
}

/** Result of resolving the effective policy for a working directory. */
export interface ResolvedPolicy {
  readonly policy: Policy
  /** The policy file that contributed rules, when one was read. */
  readonly filePath?: string
  /** Set when a policy file exists but could not be read or parsed. */
  readonly error?: string
}

/**
 * Resolve, cache, and evaluate the policy for a working directory.
 *
 * Config rules are validated eagerly by {@link createPolicyRuntime}, so a typo
 * is a plugin load failure. A policy file is re-read when its modification time
 * changes, so editing rules does not require a restart.
 */
export class PolicyRuntime {
  private cached: { path: string; mtimeMs: number; policy: Policy } | undefined

  constructor(
    private readonly config: PolicyRuntimeConfig,
    private readonly cwd: () => string,
    private readonly configPolicy: Policy,
  ) {}

  /** Find the policy file that applies to the current directory, if any. */
  private policyPath(): string | undefined {
    if (this.config.policyFile) return this.config.policyFile
    if (this.config.discoverPolicy === false) return undefined
    return discoverPolicyPath(this.cwd())
  }

  /** Resolve the effective policy, reading and caching a file when present. */
  async resolve(): Promise<ResolvedPolicy> {
    const found = this.policyPath()
    if (!found) return { policy: this.configPolicy }

    let mtimeMs: number
    try {
      mtimeMs = statSync(found).mtimeMs
    } catch (error) {
      return { policy: this.configPolicy, filePath: found, error: `cannot read ${found}: ${(error as Error).message}` }
    }

    let filePolicy: Policy
    if (this.cached && this.cached.path === found && this.cached.mtimeMs === mtimeMs) {
      filePolicy = this.cached.policy
    } else {
      try {
        filePolicy = await parsePolicyFile(found)
        this.cached = { path: found, mtimeMs, policy: filePolicy }
      } catch (error) {
        const message = error instanceof PolicyError ? error.message : (error as Error).message
        return { policy: this.configPolicy, filePath: found, error: message }
      }
    }

    return {
      policy: {
        rules: [...this.configPolicy.rules, ...filePolicy.rules],
        hostExecutables: [...this.configPolicy.hostExecutables, ...filePolicy.hostExecutables],
      },
      filePath: found,
    }
  }

  /** Resolve and evaluate one command. */
  async evaluate(command: string): Promise<{ verdict: Verdict; resolved: ResolvedPolicy }> {
    const resolved = await this.resolve()
    const options: EvaluateOptions = {
      resolveHostExecutables: this.config.resolveHostExecutables === true,
    }
    return { verdict: evaluatePolicy(resolved.policy, command, options), resolved }
  }

  /** Whether any rule exists at all, so callers can skip the pipeline entirely. */
  async hasRules(): Promise<boolean> {
    const resolved = await this.resolve()
    return resolved.policy.rules.length > 0
  }
}

/**
 * Create a runtime, validating config-supplied rules immediately.
 *
 * Throws {@link PolicyError} for a malformed config policy: a bad policy should
 * fail the plugin load, not surface as a runtime denial.
 */
export function createPolicyRuntime(
  config: PolicyRuntimeConfig,
  cwd: () => string,
): PolicyRuntime {
  return new PolicyRuntime(config, cwd, parsePolicy(config.policy))
}
