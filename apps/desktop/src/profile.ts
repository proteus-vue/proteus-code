/**
 * Profile management for the proteus-code desktop app.
 *
 * The desktop app owns its own Harness home (`~/.proteus-code`) so it never
 * disturbs a user's `dsh` CLI state. On first launch it writes a profile whose
 * bundle stack is dsh-base + dsh-web-app + this repository's Proteus bundle, then
 * installs the bundle from the built package directory with the bundled DSH CLI.
 *
 * @module @proteus-code/desktop/profile
 */

import { spawn } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import path from 'node:path'

/** The bundles every proteus-code profile stacks, in layer order. */
export const PROFILE_BUNDLES = [
  '@deepseek-ai/dsh-base',
  '@deepseek-ai/dsh-web-app',
  '@proteus-code/dsh-proteus-code',
] as const

/** Name of the profile the app owns. */
export const PROFILE_NAME = 'proteus-code'

/** Name of the bundle package as published in the workspace. */
const BUNDLE_NAME = '@proteus-code/dsh-proteus-code'

/** Resolved paths the desktop process needs. */
export interface DesktopPaths {
  /** Root of the proteus-code repository checkout. */
  repoRoot: string
  /** Harness home owned by the desktop app. */
  dshHome: string
  /** Absolute path to the built Proteus bundle package. */
  bundleDir: string
  /** Absolute path to the DSH CLI's JavaScript entry point. */
  dshEntry: string
}

/** Node major version DSH requires. */
const REQUIRED_NODE_MAJOR = 22

/** Resolve the repository root by walking up from a starting directory. */
export function findRepoRoot(startDir: string): string {
  let current = path.resolve(startDir)
  for (;;) {
    if (existsSync(path.join(current, 'pnpm-workspace.yaml'))) return current
    const parent = path.dirname(current)
    if (parent === current) {
      throw new Error(`proteus-code: could not find the repository root above ${startDir}`)
    }
    current = parent
  }
}

/**
 * Find a Node ≥22 executable.
 *
 * DSH requires Node 22.19+. The system `node` may be older, so an explicit
 * override wins, then known version-manager layout under the home directory.
 * When none is found the caller runs Node inside Electron's bundled runtime
 * (`ELECTRON_RUN_AS_NODE`), which also satisfies the requirement.
 */
export function resolveNode(): { command: string; env: NodeJS.ProcessEnv } | undefined {
  const override = process.env.PROTEUS_CODE_NODE
  if (override && existsSync(override)) return { command: override, env: {} }

  const candidates: string[] = []
  const nvmDir = path.join(homedir(), '.nvm/versions/node')
  if (existsSync(nvmDir)) {
    for (const entry of readdirSync(nvmDir)) {
      const major = Number.parseInt(entry.replace(/^v/, ''), 10)
      if (Number.isFinite(major) && major >= REQUIRED_NODE_MAJOR) {
        candidates.push(path.join(nvmDir, entry, 'bin/node'))
      }
    }
    candidates.sort().reverse()
  }
  for (const candidate of candidates) {
    if (existsSync(candidate)) return { command: candidate, env: {} }
  }

  // Fall back to Node inside Electron: `process.execPath` IS Electron here.
  if (process.versions.electron) {
    return { command: process.execPath, env: { ELECTRON_RUN_AS_NODE: '1' } }
  }
  return undefined
}

/** Resolve the DSH CLI JavaScript entry, preferring an explicit override. */
export function resolveDshEntry(repoRoot: string): string {
  const override = process.env.PROTEUS_CODE_DSH
  const candidates = [
    override,
    path.join(repoRoot, 'node_modules/@deepseek-ai/dsh/lib/bin.js'),
    path.join(repoRoot, 'node_modules/@deepseek-ai/dsh/bin/lib.js'),
  ].filter((candidate): candidate is string => Boolean(candidate))
  for (const candidate of candidates) {
    if (existsSync(candidate)) return candidate
  }
  throw new Error(
    `proteus-code: DSH CLI not found. Run \`pnpm install\` in ${repoRoot}, ` +
      `or set PROTEUS_CODE_DSH to the dsh entry script.`,
  )
}

/** Resolve every path the app needs, honoring environment overrides. */
export function resolvePaths(appDir: string): DesktopPaths {
  const repoRoot = process.env.PROTEUS_CODE_REPO ?? findRepoRoot(appDir)
  const dshHome = process.env.PROTEUS_CODE_HOME ?? path.join(homedir(), '.proteus-code')
  const bundleDir = path.join(repoRoot, 'packages/proteus-code-bundle')
  const dshEntry = resolveDshEntry(repoRoot)
  return { repoRoot, dshHome, bundleDir, dshEntry }
}

/** The profile manifest written on first launch. */
function profileManifest(): string {
  return `${JSON.stringify(
    {
      name: `dsh-profile-${PROFILE_NAME}`,
      private: true,
      dependencies: {},
      dsh: { profile: { bundles: [...PROFILE_BUNDLES], patchReload: 'live' } },
    },
    null,
    2,
  )}\n`
}

/**
 * The profile's pnpm workspace file. `dsh plugin add` forwards to pnpm with
 * `-w`, which requires a workspace, so a hand-written profile must declare one.
 * These values mirror what the DSH profile initializer writes.
 */
const PROFILE_WORKSPACE = 'packages:\n  - .\n\nnodeLinker: hoisted\nautoInstallPeers: false\n'

/** Whether the profile already stacks this repository's bundle. */
export function profileIsReady(dshHome: string): boolean {
  const profileDir = path.join(dshHome, 'profiles', PROFILE_NAME)
  if (!existsSync(path.join(profileDir, 'pnpm-workspace.yaml'))) return false
  const manifestPath = path.join(profileDir, 'package.json')
  if (!existsSync(manifestPath)) return false
  try {
    const manifest = JSON.parse(readFileSync(manifestPath, 'utf8')) as {
      dsh?: { profile?: { bundles?: string[] } }
    }
    const bundles = manifest.dsh?.profile?.bundles ?? []
    return PROFILE_BUNDLES.every((bundle) => bundles.includes(bundle))
  } catch {
    return false
  }
}

/** Run a command to completion, capturing stdio. */
function run(
  command: string,
  args: string[],
  options: { cwd: string; env: NodeJS.ProcessEnv },
): Promise<string> {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { ...options, stdio: ['ignore', 'pipe', 'pipe'] })
    let output = ''
    child.stdout?.on('data', (chunk) => (output += chunk))
    child.stderr?.on('data', (chunk) => (output += chunk))
    child.on('error', reject)
    child.on('exit', (code) => {
      if (code === 0) resolve(output)
      else reject(new Error(`${command} ${args.join(' ')} exited ${code}\n${output}`))
    })
  })
}

/**
 * Ensure the desktop profile exists and stacks the Proteus bundle.
 *
 * Idempotent, and self-updating: when the built bundle is newer than the copy
 * installed in the profile, the old copy is removed first so pnpm re-materializes
 * it. The bundle is installed as a `file:` dependency rather than a bare path,
 * because a bare path becomes a symlink to the repository and would resolve DSH
 * packages from the repository's isolated store; a `file:` install lands inside
 * the profile, where the harness's shared module fallback serves them.
 */
export async function ensureProfile(paths: DesktopPaths, log: (message: string) => void): Promise<void> {
  const profileDir = path.join(paths.dshHome, 'profiles', PROFILE_NAME)
  mkdirSync(profileDir, { recursive: true })

  const node = resolveNode()
  if (!node) {
    throw new Error(
      'proteus-code: no Node ≥22 runtime found. Install Node 22.19+, or set PROTEUS_CODE_NODE.',
    )
  }
  const env = {
    ...process.env,
    ...node.env,
    DSH_HOME: paths.dshHome,
    npm_config_yes: 'true',
  }
  const dsh = (args: string[]) => run(node.command, [paths.dshEntry, ...args], {
    cwd: paths.repoRoot,
    env,
  })

  if (!profileIsReady(paths.dshHome)) {
    log('[proteus-code] initializing the desktop Harness profile…')
    writeFileSync(path.join(profileDir, 'package.json'), profileManifest(), 'utf8')
    writeFileSync(path.join(profileDir, 'pnpm-workspace.yaml'), PROFILE_WORKSPACE, 'utf8')
  }

  const distEntry = path.join(paths.bundleDir, 'dist/index.js')
  if (!existsSync(distEntry)) {
    throw new Error(
      `proteus-code: the Proteus bundle is not built at ${paths.bundleDir}/dist. ` +
        `Run \`pnpm build:bundle\`.`,
    )
  }

  if (bundleNeedsRefresh(paths)) {
    log('[proteus-code] refreshing the installed Proteus bundle…')
    await dsh(['plugin', '--profile', PROFILE_NAME, 'remove', '-w', BUNDLE_NAME]).catch(() => {})
  }

  log('[proteus-code] installing the Proteus bundle into the profile…')
  await dsh([
    'plugin',
    '--profile',
    PROFILE_NAME,
    'add',
    '-w',
    `file:${paths.bundleDir}`,
  ])
}

/** Whether the built bundle is newer than what the profile has installed. */
function bundleNeedsRefresh(paths: DesktopPaths): boolean {
  const installed = path.join(
    paths.dshHome,
    'profiles',
    PROFILE_NAME,
    'node_modules',
    ...BUNDLE_NAME.split('/'),
    'dist/index.js',
  )
  if (!existsSync(installed)) return false
  try {
    return statSync(path.join(paths.bundleDir, 'dist/index.js')).mtimeMs > statSync(installed).mtimeMs
  } catch {
    return false
  }
}

/** Build the argv that launches the harness, and the environment it needs. */
export function harnessLaunch(
  paths: DesktopPaths,
  extraArgs: string[],
): { command: string; args: string[]; env: NodeJS.ProcessEnv } {
  const node = resolveNode()
  if (!node) {
    throw new Error(
      'proteus-code: no Node ≥22 runtime found. Install Node 22.19+, or set PROTEUS_CODE_NODE.',
    )
  }
  return {
    command: node.command,
    args: [paths.dshEntry, ...extraArgs],
    env: { ...process.env, ...node.env, DSH_HOME: paths.dshHome, FORCE_COLOR: '0' },
  }
}
