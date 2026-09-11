import { existsSync } from 'node:fs'
import path from 'node:path'
import { describe, expect, it } from 'vitest'
import { createProteusTools } from '../src/tools.ts'

/**
 * Integration proof that the tools drive the real Proteus CLI.
 *
 * Opt-in: set PROTEUS_CODE_TEST_CHECKOUT to a Proteus checkout that has a built
 * CLI, e.g. `PROTEUS_CODE_TEST_CHECKOUT=/path/to/proteus pnpm test`. The test
 * skips otherwise, so the default suite stays hermetic and fast.
 */
const checkout = process.env.PROTEUS_CODE_TEST_CHECKOUT
const bin = checkout ? path.join(checkout, 'node_modules/.bin/proteus') : undefined
const runnable = Boolean(checkout && bin && existsSync(bin))

const describeIntegration = runnable ? describe : describe.skip

/** Locate a registered tool by name from the factory output. */
function toolNamed(tools: ReturnType<typeof createProteusTools>, name: string) {
  const tool = tools.find((t) => (t as { name: string }).name === name)
  if (!tool) throw new Error(`tool not registered: ${name}`)
  return tool as unknown as {
    execute(args: unknown, exec: { signal: AbortSignal }): Promise<unknown>
  }
}

describeIntegration('real Proteus CLI integration', () => {
  const tools = createProteusTools({
    config: { proteusRoot: checkout as string, timeoutMs: 180_000 },
    cwd: () => checkout as string,
  })
  const exec = { signal: new AbortController().signal }

  it('runs `proteus rules` and returns real rule output', async () => {
    const value = (await toolNamed(tools, 'proteus_rules').execute({}, exec)) as {
      ok: boolean
      stdout: string
      exitCode: number | null
    }
    expect(value.ok, `exit=${value.exitCode}`).toBe(true)
    // The rule catalog names at least one real registered transform.
    expect(value.stdout).toMatch(/tag\/|semantic\//)
  }, 180_000)

  it('runs `proteus health` and reports structured output', async () => {
    const value = (await toolNamed(tools, 'proteus_health').execute({ dir: '.' }, exec)) as {
      command: string
      stdout: string
      stderr: string
      cwd: string
    }
    expect(value.command).toContain('health')
    // A health run either reports findings or a summary; it must produce output.
    expect((value.stdout + value.stderr).trim().length).toBeGreaterThan(0)
  }, 180_000)

  it('rejects an unknown escape-hatch subcommand without spawning', async () => {
    await expect(
      toolNamed(tools, 'proteus_cli').execute({ subcommand: 'rm -rf /' }, exec),
    ).rejects.toThrow(/unknown subcommand/)
  })
})
