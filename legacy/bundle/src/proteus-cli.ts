/**
 * Proteus CLI bridge.
 *
 * Resolves a Proteus checkout and runs `proteus <subcommand> ...` against it.
 * Execution prefers the harness shell service (`ctx.shell`), which applies the
 * session's sandbox and approval policy; when that seam is absent it falls back
 * to a local `child_process` spawn so the tools still work in minimal profiles.
 *
 * @module @proteus-code/dsh-proteus-code/proteus-cli
 */

import { execFile } from 'node:child_process'
import { existsSync, statSync } from 'node:fs'
import path from 'node:path'

/** One fully resolved CLI invocation. */
export interface CliInvocation {
  command: string
  workdir: string
  timeoutMs?: number | undefined
  signal?: AbortSignal | undefined
}

/** Normalized outcome of one CLI run, independent of the execution backend. */
export interface CliResult {
  ok: boolean
  exitCode: number | null
  signal: string | null
  command: string
  cwd: string
  stdout: string
  stderr: string
  truncated: boolean
  durationMs: number
}

/** The execution seam the tools depend on. */
export interface CliRunner {
  /** Run one resolved invocation and normalize its outcome. */
  run(invocation: CliInvocation): Promise<CliResult>
}

/** Structural view of `ctx.shell` (`@deepseek-ai/dsh-shell`). */
export interface ShellSeam {
  resolve(request: {
    command: string
    workdir?: string
    timeoutMs?: number
    signal?: AbortSignal
  }): unknown
  run(spec: unknown): Promise<{
    exitCode: number | null
    signal: string | null
    timedOut: boolean
    aborted: boolean
    stdout: { text: string; truncated: boolean }
    stderr: { text: string; truncated: boolean }
  }>
}

/** Quote one argv token for POSIX shells. */
export function shellQuote(token: string): string {
  if (token.length > 0 && /^[A-Za-z0-9_@%+=:,./-]+$/.test(token)) return token
  return `'${token.replaceAll("'", `'\\''`)}'`
}

/** Render an argv vector as a single shell command string. */
export function toCommandString(argv: readonly string[]): string {
  return argv.map(shellQuote).join(' ')
}

/** Candidate locations that mark a Proteus checkout root. */
const ROOT_MARKERS = ['packages/cli/src/index.ts', 'proteus.config.ts', 'proteus.config.js']

/** Walk upward from `startDir` looking for a Proteus checkout. */
export function findProteusRoot(startDir: string): string | undefined {
  let current = path.resolve(startDir)
  for (;;) {
    for (const marker of ROOT_MARKERS) {
      if (existsSync(path.join(current, marker))) return current
    }
    const parent = path.dirname(current)
    if (parent === current) return undefined
    current = parent
  }
}

function isFile(target: string): boolean {
  try {
    return statSync(target).isFile()
  } catch {
    return false
  }
}

/**
 * Resolve the argv prefix that invokes the Proteus CLI.
 *
 * Preference order: an explicit command override, the built `proteus` binary in
 * the checkout, the TypeScript entry via `tsx`, then a compiled `dist` entry.
 * The returned argv excludes the Proteus subcommand and its arguments.
 */
export function resolveProteusArgv(root: string, override?: string): string[] {
  if (override && override.trim().length > 0) return splitCommand(override)
  const bin = path.join(root, 'node_modules/.bin/proteus')
  if (isFile(bin)) return [bin]
  const tsEntry = path.join(root, 'packages/cli/src/index.ts')
  if (isFile(tsEntry)) return [process.execPath, '--import', 'tsx/esm', tsEntry]
  for (const candidate of ['dist/cli/index.js', 'packages/cli/dist/index.js']) {
    const built = path.join(root, candidate)
    if (isFile(built)) return [process.execPath, built]
  }
  throw new Error(
    `proteus-code: no Proteus CLI found under ${root}. ` +
      `Expected node_modules/.bin/proteus, packages/cli/src/index.ts, or a built dist entry. ` +
      `Set the "proteusCommand" plugin option to override.`,
  )
}

/** Split a user-supplied command string on whitespace, honoring simple quoting. */
function splitCommand(input: string): string[] {
  const tokens: string[] = []
  const pattern = /'([^']*)'|"([^"]*)"|(\S+)/g
  let match: RegExpExecArray | null
  while ((match = pattern.exec(input)) !== null) {
    tokens.push(match[1] ?? match[2] ?? match[3] ?? '')
  }
  return tokens
}

/** Runner backed by `ctx.shell`, honoring the session sandbox policy. */
class ShellCliRunner implements CliRunner {
  constructor(private readonly shell: ShellSeam) {}

  async run(invocation: CliInvocation): Promise<CliResult> {
    const started = Date.now()
    const spec = this.shell.resolve({
      command: invocation.command,
      workdir: invocation.workdir,
      timeoutMs: invocation.timeoutMs,
      signal: invocation.signal,
    })
    const result = await this.shell.run(spec)
    const exitCode = result.exitCode
    return {
      ok: exitCode === 0,
      exitCode,
      signal: result.signal ?? null,
      command: invocation.command,
      cwd: invocation.workdir,
      stdout: result.stdout.text,
      stderr: result.stderr.text,
      truncated: result.stdout.truncated || result.stderr.truncated,
      durationMs: Date.now() - started,
    }
  }
}

/** Runner backed by `child_process.execFile`, for profiles without a shell seam. */
class LocalCliRunner implements CliRunner {
  async run(invocation: CliInvocation): Promise<CliResult> {
    const started = Date.now()
    const argv = splitCommand(invocation.command)
    const file = argv[0]
    if (!file) throw new Error('proteus-code: empty command')
    return await new Promise<CliResult>((resolve) => {
      const child = execFile(
        file,
        argv.slice(1),
        {
          cwd: invocation.workdir,
          timeout: invocation.timeoutMs ?? 120_000,
          maxBuffer: 16 * 1024 * 1024,
          signal: invocation.signal,
          env: { ...process.env, FORCE_COLOR: '0', NO_COLOR: '1' },
        },
        (error, stdout, stderr) => {
          const exitCode =
            error && typeof (error as NodeJS.ErrnoException).code === 'number'
              ? ((error as unknown as { code: number }).code as number)
              : error
                ? null
                : 0
          resolve({
            ok: exitCode === 0,
            exitCode,
            signal: null,
            command: invocation.command,
            cwd: invocation.workdir,
            stdout: String(stdout ?? ''),
            stderr: String(stderr ?? ''),
            truncated: false,
            durationMs: Date.now() - started,
          })
        },
      )
      child.on('error', (error) => {
        resolve({
          ok: false,
          exitCode: null,
          signal: null,
          command: invocation.command,
          cwd: invocation.workdir,
          stdout: '',
          stderr: String(error.message ?? error),
          truncated: false,
          durationMs: Date.now() - started,
        })
      })
    })
  }
}

/** Choose the strongest available execution backend. */
export function createCliRunner(shell?: ShellSeam | undefined): CliRunner {
  return shell ? new ShellCliRunner(shell) : new LocalCliRunner()
}

/** Everything a Proteus tool needs to turn arguments into one CLI run. */
export interface ProteusInvocation {
  argvPrefix: string[]
  root: string
}

/** Resolve the invocation for a given working directory. */
export function resolveInvocation(
  options: { proteusRoot?: string | undefined; proteusCommand?: string | undefined },
  cwd: string,
): ProteusInvocation {
  const root = options.proteusRoot
    ? path.resolve(options.proteusRoot)
    : findProteusRoot(cwd)
  if (!root) {
    throw new Error(
      `proteus-code: could not locate a Proteus checkout from ${cwd}. ` +
        `Run inside the project, or set the "proteusRoot" plugin option.`,
    )
  }
  if (!existsSync(root)) {
    throw new Error(`proteus-code: proteusRoot does not exist: ${root}`)
  }
  return { argvPrefix: resolveProteusArgv(root, options.proteusCommand), root }
}
