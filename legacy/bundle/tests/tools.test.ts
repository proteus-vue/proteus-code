import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { afterAll, describe, expect, it } from 'vitest'
import { createProteusTools } from '../src/tools.ts'

/**
 * The tool-to-argument mapping is the contract most likely to drift when the
 * Proteus CLI changes. These tests assert the exact argv each tool builds, using
 * a stub shell that records the command instead of running it.
 */

/** A real directory that looks like a Proteus checkout, so resolution succeeds. */
const fakeRoot = mkdtempSync(path.join(tmpdir(), 'proteus-code-tools-'))
writeFileSync(path.join(fakeRoot, 'proteus.config.ts'), 'export default {}\n')
afterAll(() => rmSync(fakeRoot, { recursive: true, force: true }))

interface Recorded {
  command: string
  workdir: string
}

/** A shell seam that records commands and returns an empty success. */
function recordingShell(commands: Recorded[]) {
  return {
    resolve: (request: { command: string; workdir?: string }) => {
      commands.push({ command: request.command, workdir: request.workdir ?? '' })
      return request
    },
    run: async () => ({
      exitCode: 0,
      signal: null,
      timedOut: false,
      aborted: false,
      stdout: { text: '', truncated: false },
      stderr: { text: '', truncated: false },
    }),
  }
}

/** Invoke one tool and return the command the shell received. */
async function commandFor(name: string, args: unknown): Promise<string> {
  const commands: Recorded[] = []
  const tools = createProteusTools({
    config: { proteusCommand: 'proteus', proteusRoot: fakeRoot },
    shell: recordingShell(commands),
    cwd: () => fakeRoot,
  })
  const tool = tools.find((t) => (t as { name: string }).name === name) as unknown as {
    execute(args: unknown, exec: { signal: AbortSignal }): Promise<unknown>
  }
  await tool.execute(args, { signal: new AbortController().signal })
  return commands[0]?.command ?? ''
}

describe('tool argv mapping', () => {
  it('health passes a directory only when given', async () => {
    expect(await commandFor('proteus_health', {})).toBe('proteus health')
    expect(await commandFor('proteus_health', { dir: 'examples/app' })).toBe(
      'proteus health examples/app',
    )
  })

  it('check maps strict toggles to the negation flags', async () => {
    expect(await commandFor('proteus_check', {})).toBe('proteus check')
    expect(await commandFor('proteus_check', { strictCss: false, strictCli: false })).toBe(
      'proteus check --no-strict-css --no-strict-cli',
    )
    // A true value must NOT emit the negated flag.
    expect(await commandFor('proteus_check', { strictCss: true })).toBe('proteus check')
  })

  it('build always states the target', async () => {
    expect(await commandFor('proteus_build', { target: 'web' })).toBe(
      'proteus build . --target web',
    )
    expect(await commandFor('proteus_build', { target: 'skyline', out: 'dist/mp', debug: true })).toBe(
      'proteus build . --target skyline --out dist/mp --debug',
    )
    expect(await commandFor('proteus_build', { target: 'all', compiler: 'rust' })).toBe(
      'proteus build . --target all --compiler rust',
    )
  })

  it('explain and rules map positional and phase arguments', async () => {
    expect(await commandFor('proteus_explain', { target: 'src/pages/home.vue' })).toBe(
      'proteus explain src/pages/home.vue',
    )
    expect(await commandFor('proteus_rules', { phase: 'style' })).toBe('proteus rules style')
  })

  it('audit passes the kind and optional flags', async () => {
    expect(await commandFor('proteus_audit', { kind: 'coverage' })).toBe('proteus audit coverage')
    expect(await commandFor('proteus_audit', { kind: 'module', dir: '.', dist: 'dist/web' })).toBe(
      'proteus audit module . --dist dist/web',
    )
  })

  it('conformance maps repo, backend, only, and demo', async () => {
    expect(await commandFor('proteus_conformance', { repo: '.' })).toBe(
      'proteus conformance --repo .',
    )
    expect(
      await commandFor('proteus_conformance', { backend: './b.js#factory', only: 'C-03' }),
    ).toBe("proteus conformance --backend './b.js#factory' --only C-03")
  })

  it('migrate_mp maps dry-run', async () => {
    expect(await commandFor('proteus_migrate_mp', { target: 'src/legacy', dryRun: true })).toBe(
      'proteus migrate mp src/legacy --dry-run',
    )
  })

  it('cli escape hatch passes through allowlisted subcommands verbatim', async () => {
    expect(await commandFor('proteus_cli', { subcommand: 'css:check', args: ['src', '--fix'] })).toBe(
      'proteus css:check src --fix',
    )
  })

  it('quotes arguments containing spaces', async () => {
    expect(await commandFor('proteus_health', { dir: 'my projects/app' })).toBe(
      "proteus health 'my projects/app'",
    )
  })
})
