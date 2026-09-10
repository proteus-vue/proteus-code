import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { afterAll, describe, expect, it } from 'vitest'
import {
  createCliRunner,
  findProteusRoot,
  resolveInvocation,
  resolveProteusArgv,
  shellQuote,
  toCommandString,
  type ShellSeam,
} from '../src/proteus-cli.ts'

const scratch = mkdtempSync(path.join(tmpdir(), 'proteus-code-test-'))
afterAll(() => rmSync(scratch, { recursive: true, force: true }))

function makeCheckout(layout: 'source' | 'built', dirName: string): string {
  const root = path.join(scratch, dirName)
  mkdirSync(root, { recursive: true })
  writeFileSync(path.join(root, 'proteus.config.ts'), 'export default {}\n')
  if (layout === 'source') {
    mkdirSync(path.join(root, 'packages/cli/src'), { recursive: true })
    writeFileSync(path.join(root, 'packages/cli/src/index.ts'), '// entry\n')
  } else {
    mkdirSync(path.join(root, 'dist/cli'), { recursive: true })
    writeFileSync(path.join(root, 'dist/cli/index.js'), '// built\n')
  }
  return root
}

describe('shell quoting', () => {
  it('leaves safe tokens unquoted', () => {
    expect(shellQuote('proteus')).toBe('proteus')
    expect(shellQuote('--target')).toBe('--target')
    expect(shellQuote('css:check')).toBe('css:check')
    expect(shellQuote('/a/b/c.ts')).toBe('/a/b/c.ts')
  })

  it('quotes tokens with spaces or shell metacharacters', () => {
    expect(shellQuote('a b')).toBe("'a b'")
    expect(shellQuote('a;rm -rf /')).toBe("'a;rm -rf /'")
    expect(shellQuote('$(whoami)')).toBe("'$(whoami)'")
  })

  it('escapes embedded single quotes', () => {
    expect(shellQuote("it's")).toBe("'it'\\''s'")
  })

  it('joins argv into one command line', () => {
    expect(toCommandString(['proteus', 'build', '.', '--target', 'web'])).toBe(
      'proteus build . --target web',
    )
  })
})

describe('checkout discovery', () => {
  it('finds a Proteus root by walking upward', () => {
    const root = makeCheckout('source', 'walk-root')
    const nested = path.join(root, 'examples/pages/deep')
    mkdirSync(nested, { recursive: true })
    expect(findProteusRoot(nested)).toBe(root)
  })

  it('returns undefined outside any checkout', () => {
    const lonely = path.join(scratch, 'no-checkout')
    mkdirSync(lonely, { recursive: true })
    expect(findProteusRoot(lonely)).toBeUndefined()
  })
})

describe('argv resolution', () => {
  it('prefers the TypeScript entry for a source checkout', () => {
    const root = makeCheckout('source', 'argv-source')
    const argv = resolveProteusArgv(root)
    expect(argv[0]).toBe(process.execPath)
    expect(argv).toContain('tsx/esm')
    expect(argv.at(-1)).toBe(path.join(root, 'packages/cli/src/index.ts'))
  })

  it('uses a built dist entry when there is no source entry', () => {
    const root = makeCheckout('built', 'argv-built')
    const argv = resolveProteusArgv(root)
    expect(argv[0]).toBe(process.execPath)
    expect(argv.at(-1)).toBe(path.join(root, 'dist/cli/index.js'))
  })

  it('honors an explicit command override', () => {
    const root = makeCheckout('source', 'argv-override')
    expect(resolveProteusArgv(root, 'my-proteus --flag')).toEqual(['my-proteus', '--flag'])
  })

  it('fails loudly when no CLI can be found', () => {
    const root = path.join(scratch, 'empty-root')
    mkdirSync(root, { recursive: true })
    expect(() => resolveProteusArgv(root)).toThrow(/no Proteus CLI found/)
  })
})

describe('invocation resolution', () => {
  it('resolves root and argv from an explicit proteusRoot', () => {
    const root = makeCheckout('source', 'invoke-explicit')
    const invocation = resolveInvocation({ proteusRoot: root }, '/')
    expect(invocation.root).toBe(root)
    expect(invocation.argvPrefix.length).toBeGreaterThan(0)
  })

  it('fails with guidance when the checkout cannot be found', () => {
    const lonely = path.join(scratch, 'invoke-lonely')
    mkdirSync(lonely, { recursive: true })
    expect(() => resolveInvocation({}, lonely)).toThrow(/could not locate a Proteus checkout/)
  })
})

describe('runner selection', () => {
  it('routes through the shell seam when provided', async () => {
    const calls: string[] = []
    const shell: ShellSeam = {
      resolve: (request) => {
        calls.push(`resolve:${request.command}`)
        return request
      },
      run: async () => {
        calls.push('run')
        return {
          exitCode: 0,
          signal: null,
          timedOut: false,
          aborted: false,
          stdout: { text: 'ok', truncated: false },
          stderr: { text: '', truncated: false },
        }
      },
    }
    const result = await createCliRunner(shell).run({ command: 'proteus health', workdir: '/' })
    expect(result.ok).toBe(true)
    expect(result.stdout).toBe('ok')
    expect(calls).toEqual(['resolve:proteus health', 'run'])
  })

  it('falls back to a local process when no shell seam exists', async () => {
    const result = await createCliRunner(undefined).run({
      command: `${process.execPath} -e "process.stdout.write('local-ok')"`,
      workdir: scratch,
    })
    expect(result.ok).toBe(true)
    expect(result.stdout).toContain('local-ok')
  })
})
