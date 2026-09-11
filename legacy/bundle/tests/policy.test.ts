import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { afterAll, describe, expect, it } from 'vitest'
import {
  discoverPolicyPath,
  evaluatePolicy,
  matchesPrefix,
  parsePolicy,
  parsePolicyFile,
  PolicyError,
  tokenizeCommand,
  type Policy,
} from '../src/policy.ts'
import { commandForCall, explainVerdict, installPolicyGuard } from '../src/policy-guard.ts'
import { createPolicyRuntime } from '../src/policy-runtime.ts'
import { createPolicyTool } from '../src/policy-tool.ts'

const scratch = mkdtempSync(path.join(tmpdir(), 'proteus-code-policy-'))
afterAll(() => rmSync(scratch, { recursive: true, force: true }))

/** Build a policy from the compact rule list used across these tests. */
function policy(rules: unknown, hostExecutables?: unknown): Policy {
  return parsePolicy({ rules, ...(hostExecutables ? { hostExecutables } : {}) })
}

describe('tokenization', () => {
  it('splits on whitespace', () => {
    expect(tokenizeCommand('git status --short')).toEqual(['git', 'status', '--short'])
  })

  it('honors quoting so a quoted path is one token', () => {
    expect(tokenizeCommand('cat "my file.txt"')).toEqual(['cat', 'my file.txt'])
    expect(tokenizeCommand("rm 'a b'")).toEqual(['rm', 'a b'])
  })

  it('honors backslash escapes outside quotes', () => {
    expect(tokenizeCommand('echo a\\ b')).toEqual(['echo', 'a b'])
  })

  it('collapses repeated whitespace and trims', () => {
    expect(tokenizeCommand('  ls   -la  ')).toEqual(['ls', '-la'])
  })

  it('returns nothing for an empty command', () => {
    expect(tokenizeCommand('   ')).toEqual([])
  })
})

describe('prefix matching (Codex semantics)', () => {
  it('matches a literal prefix', () => {
    const rule = policy([{ pattern: ['git', 'status'], decision: 'allow' }]).rules[0]!
    expect(matchesPrefix(rule, ['git', 'status'])).toBe(true)
    expect(matchesPrefix(rule, ['git', 'status', '--short'])).toBe(true)
    expect(matchesPrefix(rule, ['git', 'log'])).toBe(false)
  })

  it('treats a list entry as alternatives', () => {
    const rule = policy([{ pattern: ['rm', ['-r', '-rf', '-f']] }]).rules[0]!
    expect(matchesPrefix(rule, ['rm', '-r'])).toBe(true)
    expect(matchesPrefix(rule, ['rm', '-rf'])).toBe(true)
    expect(matchesPrefix(rule, ['rm', '-f'])).toBe(true)
    expect(matchesPrefix(rule, ['rm', '-q'])).toBe(false)
  })

  it('does not match when the command is shorter than the pattern', () => {
    const rule = policy([{ pattern: ['git', 'status', '--short'] }]).rules[0]!
    expect(matchesPrefix(rule, ['git', 'status'])).toBe(false)
  })
})

describe('self-testing rules', () => {
  it('accepts a correct rule whose examples hold', () => {
    expect(() =>
      policy([
        {
          pattern: ['git', 'push'],
          decision: 'prompt',
          match: ['git push', ['git', 'push', 'origin']],
          notMatch: ['git pull'],
        },
      ]),
    ).not.toThrow()
  })

  it('rejects a rule whose match example does not match', () => {
    expect(() =>
      policy([{ pattern: ['git', 'push'], match: ['git pull'] }]),
    ).toThrow(/match.*does not match the pattern/)
  })

  it('rejects a rule whose notMatch example does match', () => {
    expect(() =>
      policy([{ pattern: ['git', 'push'], notMatch: ['git push --force'] }]),
    ).toThrow(/notMatch.*unexpectedly matches/)
  })
})

describe('policy validation', () => {
  it('rejects an unknown decision', () => {
    expect(() => policy([{ pattern: ['ls'], decision: 'maybe' }])).toThrow(PolicyError)
  })

  it('defaults the decision to allow, as Codex does', () => {
    expect(policy([{ pattern: ['ls'] }]).rules[0]!.decision).toBe('allow')
  })

  it('rejects an alternatives list in the first position', () => {
    expect(() => policy([{ pattern: [['git', 'hg'], 'status'] }])).toThrow(
      /first `pattern` token must be a single string/,
    )
  })

  it('rejects an empty pattern', () => {
    expect(() => policy([{ pattern: [] }])).toThrow(/non-empty list of tokens/)
  })

  it('requires absolute paths in hostExecutables', () => {
    expect(() => policy([], [{ name: 'git', paths: ['relative/git'] }])).toThrow(/absolute/)
  })

  it('synthesizes a rule id when none is given', () => {
    expect(policy([{ pattern: ['rm', ['-r', '-rf']] }]).rules[0]!.id).toBe('rm [-r|-rf]')
  })

  it('treats a missing policy as empty rather than an error', () => {
    expect(parsePolicy(undefined)).toEqual({ rules: [], hostExecutables: [] })
  })
})

describe('evaluation and precedence', () => {
  it('lets the most restrictive decision win among matches', () => {
    // Codex takes the max over Allow < Prompt < Forbidden.
    const p = policy([
      { pattern: ['rm'], decision: 'allow' },
      { pattern: ['rm', '-rf'], decision: 'forbidden', justification: 'irreversible' },
    ])
    const verdict = evaluatePolicy(p, 'rm -rf /')
    expect(verdict.decision).toBe('forbidden')
    expect(verdict.justification).toBe('irreversible')
    expect(verdict.matched.map((r) => r.decision)).toEqual(['forbidden', 'allow'])
  })

  it('returns allow with no matches, and the caller then delegates', () => {
    const p = policy([{ pattern: ['ls'] }])
    expect(evaluatePolicy(p, 'echo hi')).toEqual({ decision: 'allow', matched: [] })
  })

  it('does not confuse a prefix of another command', () => {
    const p = policy([{ pattern: ['git'], decision: 'forbidden' }])
    expect(evaluatePolicy(p, 'gitleaks detect').matched).toEqual([])
  })

  it('evaluates the proteus_cli escape hatch as a proteus command line', () => {
    const p = policy([{ pattern: ['proteus', 'build'], decision: 'prompt' }])
    expect(evaluatePolicy(p, 'proteus build . --target web').decision).toBe('prompt')
  })
})

describe('host-executable pinning (Codex semantics)', () => {
  const p = policy(
    [{ pattern: ['git', 'push'], decision: 'forbidden', justification: 'push is gated' }],
    [{ name: 'git', paths: ['/usr/bin/git', '/opt/homebrew/bin/git'] }],
  )

  it('does not fall back to basename by default', () => {
    expect(evaluatePolicy(p, '/usr/bin/git push').matched).toEqual([])
  })

  it('falls back to the declared absolute path when resolution is enabled', () => {
    expect(evaluatePolicy(p, '/usr/bin/git push', { resolveHostExecutables: true }).decision).toBe(
      'forbidden',
    )
  })

  it('refuses to fall back for an undeclared path of a pinned basename', () => {
    // A same-named binary elsewhere must not inherit the rule.
    expect(
      evaluatePolicy(p, '/tmp/evil/git push', { resolveHostExecutables: true }).matched,
    ).toEqual([])
  })

  it('allows basename fallback for a basename with no hostExecutable entry', () => {
    const q = policy([{ pattern: ['cargo', 'publish'], decision: 'forbidden' }])
    expect(
      evaluatePolicy(q, '/usr/local/bin/cargo publish', { resolveHostExecutables: true }).decision,
    ).toBe('forbidden')
  })
})

describe('policy discovery', () => {
  it('finds a policy file by walking upward', () => {
    const root = path.join(scratch, 'project')
    const nested = path.join(root, 'src/pages')
    const dir = path.join(root, '.proteus-code')
    rmSync(root, { recursive: true, force: true })
    writeFileSync(path.join(createDirs(dir), 'policy.json'), '{"rules":[]}', 'utf8')
    expect(discoverPolicyPath(createDirs(nested))).toBe(path.join(dir, 'policy.json'))
  })

  it('returns undefined when no policy exists', () => {
    const lonely = createDirs(path.join(scratch, 'lonely'))
    expect(discoverPolicyPath(lonely)).toBeUndefined()
  })

  it('parses a JSON policy file', async () => {
    const file = path.join(createDirs(path.join(scratch, 'files')), 'policy.json')
    writeFileSync(file, JSON.stringify({ rules: [{ pattern: ['npm', 'publish'], decision: 'forbidden' }] }))
    const parsed = await parsePolicyFile(file)
    expect(parsed.rules[0]!.decision).toBe('forbidden')
  })

  it('reports a parse failure with the file named', async () => {
    const file = path.join(createDirs(path.join(scratch, 'bad')), 'policy.json')
    writeFileSync(file, '{ not json')
    await expect(parsePolicyFile(file)).rejects.toThrow()
  })
})

/** `mkdir -p` that returns the directory, so a test can build a path inline. */
function createDirs(dir: string): string {
  mkdirSync(dir, { recursive: true })
  return dir
}

describe('guard: which calls a policy sees', () => {
  it('judges shell tools on their command', () => {
    expect(commandForCall({ name: 'bash', arguments: { command: 'rm -rf /' } })).toBe('rm -rf /')
    expect(commandForCall({ name: 'pwsh', arguments: { command: 'Remove-Item -Recurse /' } })).toBe(
      'Remove-Item -Recurse /',
    )
  })

  it('judges the proteus escape hatch as a proteus command line', () => {
    expect(
      commandForCall({ name: 'proteus_cli', arguments: { subcommand: 'build', args: ['.'] } }),
    ).toBe('proteus build .')
  })

  it('leaves unrelated tools alone', () => {
    expect(commandForCall({ name: 'read', arguments: { path: '/etc/passwd' } })).toBeUndefined()
    expect(commandForCall({ name: 'proteus_health', arguments: {} })).toBeUndefined()
    expect(commandForCall({ name: 'bash', arguments: {} })).toBeUndefined()
  })
})

describe('guard: host integration', () => {
  /** A fake host context capturing the registered listener. */
  function host() {
    let listener:
      | ((exec: { name: string; arguments: unknown }, next: () => Promise<unknown>) => Promise<unknown>)
      | undefined
    const ctx = {
      on: (_event: string, fn: typeof listener) => {
        listener = fn
        return () => {}
      },
    }
    return {
      ctx,
      /** Run one call through the guard; `delegated` reports whether it fell through. */
      async run(name: string, args: unknown, nextResult: unknown = { kind: 'allow' }) {
        let delegated = false
        const result = await listener!({ name, arguments: args }, async () => {
          delegated = true
          return nextResult
        })
        return { result: result as { kind: string; reason?: string }, delegated }
      },
    }
  }

  it('denies a forbidden command and explains why', async () => {
    const h = host()
    installPolicyGuard(
      h.ctx as never,
      {
        runtime: createPolicyRuntime(
          {
            policy: {
              rules: [
                {
                  pattern: ['rm', ['-rf', '-fr']],
                  decision: 'forbidden',
                  justification: 'Use a targeted delete instead.',
                },
              ],
            },
            discoverPolicy: false,
          },
          () => scratch,
        ),
      },
    )
    const { result, delegated } = await h.run('bash', { command: 'rm -rf /' })
    expect(result.kind).toBe('deny')
    expect(result.reason).toContain('Use a targeted delete instead.')
    expect(delegated).toBe(false)
  })

  it('asks on a prompt command', async () => {
    const h = host()
    installPolicyGuard(h.ctx as never, {
      runtime: createPolicyRuntime(
        { policy: { rules: [{ pattern: ['git', 'push'], decision: 'prompt' }] }, discoverPolicy: false },
        () => scratch,
      ),
    })
    const { result } = await h.run('bash', { command: 'git push origin main' })
    expect(result.kind).toBe('ask')
  })

  it('delegates on an allow so the host policy still applies', async () => {
    const h = host()
    installPolicyGuard(h.ctx as never, {
      runtime: createPolicyRuntime(
        { policy: { rules: [{ pattern: ['ls'], decision: 'allow' }] }, discoverPolicy: false },
        () => scratch,
      ),
    })
    const { result, delegated } = await h.run('bash', { command: 'ls -la' }, { kind: 'allow' })
    expect(delegated).toBe(true)
    expect(result.kind).toBe('allow')
  })

  it('delegates for tools it does not judge', async () => {
    const h = host()
    installPolicyGuard(h.ctx as never, {
      runtime: createPolicyRuntime(
        { policy: { rules: [{ pattern: ['rm'], decision: 'forbidden' }] }, discoverPolicy: false },
        () => scratch,
      ),
    })
    const { delegated } = await h.run('read', { path: '/etc/passwd' })
    expect(delegated).toBe(true)
  })

  it('fails the plugin load on a malformed config policy', () => {
    expect(() =>
      createPolicyRuntime(
        { policy: { rules: [{ pattern: ['ls'], decision: 'nope' }] }, discoverPolicy: false },
        () => scratch,
      ),
    ).toThrow(PolicyError)
  })

  it('fails CLOSED when a discovered policy file is unparseable', async () => {
    const dir = createDirs(path.join(scratch, 'broken-policy'))
    writeFileSync(path.join(dir, 'proteus-code.policy.json'), '{ broken')
    const h = host()
    installPolicyGuard(h.ctx as never, { runtime: createPolicyRuntime({}, () => dir) })
    const { result, delegated } = await h.run('bash', { command: 'echo hi' })
    expect(result.kind).toBe('deny')
    expect(result.reason).toContain('failed to load')
    expect(delegated).toBe(false)
  })

  it('reads rules from a discovered policy file', async () => {
    const dir = createDirs(path.join(scratch, 'file-policy'))
    writeFileSync(
      path.join(dir, 'proteus-code.policy.json'),
      JSON.stringify({ rules: [{ pattern: ['curl'], decision: 'forbidden', justification: 'no egress' }] }),
    )
    const h = host()
    installPolicyGuard(h.ctx as never, { runtime: createPolicyRuntime({}, () => dir) })
    const { result } = await h.run('bash', { command: 'curl https://example.com' })
    expect(result.kind).toBe('deny')
    expect(result.reason).toContain('no egress')
  })

  it('re-reads the policy file after it changes', async () => {
    const dir = createDirs(path.join(scratch, 'reload-policy'))
    const file = path.join(dir, 'proteus-code.policy.json')
    writeFileSync(file, JSON.stringify({ rules: [{ pattern: ['foo'], decision: 'forbidden' }] }))
    const runtime = createPolicyRuntime({}, () => dir)
    expect((await runtime.evaluate('foo bar')).verdict.decision).toBe('forbidden')

    // Rewrite with the rule removed; the mtime bump must invalidate the cache.
    await new Promise((resolve) => setTimeout(resolve, 12))
    writeFileSync(file, JSON.stringify({ rules: [] }))
    expect((await runtime.evaluate('foo bar')).verdict.decision).toBe('allow')
  })
})

describe('policy tool (Codex `execpolicy check` equivalent)', () => {
  /** Run the tool and read back its canonical value. */
  async function check(command: string, config: Parameters<typeof createPolicyRuntime>[0], cwd: string) {
    const tool = createPolicyTool(createPolicyRuntime(config, () => cwd)) as unknown as {
      execute(args: unknown, exec: { signal: AbortSignal }): Promise<{
        decision: string
        matchedRules: string[]
        justification: string | null
        policyFile: string | null
        ruleCount: number
      }>
    }
    return await tool.execute({ command }, { signal: new AbortController().signal })
  }

  it('reports the decision and the governing rule without running anything', async () => {
    const value = await check(
      'git push origin main',
      {
        policy: {
          rules: [
            { id: 'gate-push', pattern: ['git', 'push'], decision: 'prompt', justification: 'review first' },
          ],
        },
        discoverPolicy: false,
      },
      scratch,
    )
    expect(value.decision).toBe('prompt')
    expect(value.matchedRules).toEqual(['gate-push'])
    expect(value.justification).toBe('review first')
    expect(value.ruleCount).toBe(1)
  })

  it('distinguishes "no rule matched" from "no rules at all"', async () => {
    const withRules = await check(
      'echo hi',
      { policy: { rules: [{ pattern: ['rm'], decision: 'forbidden' }] }, discoverPolicy: false },
      scratch,
    )
    expect(withRules.decision).toBe('allow')
    expect(withRules.matchedRules).toEqual([])
    expect(withRules.ruleCount).toBe(1)

    const noRules = await check('echo hi', { discoverPolicy: false }, scratch)
    expect(noRules.ruleCount).toBe(0)
  })

  it('names the policy file that applied', async () => {
    const dir = createDirs(path.join(scratch, 'tool-policy'))
    writeFileSync(
      path.join(dir, 'proteus-code.policy.json'),
      JSON.stringify({ rules: [{ id: 'no-curl', pattern: ['curl'], decision: 'forbidden' }] }),
    )
    const value = await check('curl https://x', {}, dir)
    expect(value.decision).toBe('forbidden')
    expect(value.matchedRules).toEqual(['no-curl'])
    expect(value.policyFile).toContain('proteus-code.policy.json')
  })

  it('agrees with the guard: the reported decision is what dispatch will do', async () => {
    const config = {
      policy: { rules: [{ id: 'gate', pattern: ['danger'], decision: 'forbidden', justification: 'no' }] },
      discoverPolicy: false,
    }
    const reported = await check('danger now', config, scratch)

    let listener:
      | ((exec: { name: string; arguments: unknown }, next: () => Promise<unknown>) => Promise<unknown>)
      | undefined
    installPolicyGuard(
      { on: (_e, fn) => ((listener = fn), () => {}) } as never,
      { runtime: createPolicyRuntime(config, () => scratch) },
    )
    const dispatched = (await listener!({ name: 'bash', arguments: { command: 'danger now' } }, async () => ({
      kind: 'allow',
    }))) as { kind: string }

    expect(reported.decision).toBe('forbidden')
    expect(dispatched.kind).toBe('deny')
  })
})

describe('verdict explanation', () => {
  it('names the rule and the command', () => {
    const p = policy([{ pattern: ['rm', '-rf'], decision: 'forbidden', justification: 'reason here' }])
    const verdict = evaluatePolicy(p, 'rm -rf /')
    const text = explainVerdict(verdict, 'rm -rf /')
    expect(text).toContain('Blocked')
    expect(text).toContain('rm -rf')
    expect(text).toContain('reason here')
    expect(text).toContain('rm -rf /')
  })
})
