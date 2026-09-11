/**
 * Headless proteus-code launcher.
 *
 * Resolves paths, ensures the desktop profile, then boots the harness in the
 * foreground and prints its URL. This is the non-GUI entry point: it is what the
 * desktop app does before opening a window, and it doubles as `pnpm web` and as
 * the end-to-end smoke test for profile preparation.
 *
 * @module @proteus-code/desktop/cli
 */

import { spawn } from 'node:child_process'
import path from 'node:path'
import { ensureProfile, harnessLaunch, PROFILE_NAME, resolvePaths } from './profile.js'

/**
 * Directory this module lives in, used as the search origin for the repo root.
 * `process.argv[1]` is the executable script under both Node and Electron, and
 * unlike `import.meta` it survives the CommonJS build.
 */
const here = path.dirname(path.resolve(process.argv[1] ?? process.cwd()))

/** Forwarded harness flags; everything after `--` is passed through verbatim. */
function forwardedArgs(argv: string[]): string[] {
  const separator = argv.indexOf('--')
  const passthrough = separator >= 0 ? argv.slice(separator + 1) : argv
  return passthrough.length > 0 ? passthrough : ['--no-open']
}

async function main(): Promise<void> {
  const paths = resolvePaths(here)
  console.log(`[proteus-code] repo:    ${paths.repoRoot}`)
  console.log(`[proteus-code] home:    ${paths.dshHome}`)

  await ensureProfile(paths, (message) => console.log(message))

  const launch = harnessLaunch(paths, [
    '--profile',
    PROFILE_NAME,
    ...forwardedArgs(process.argv.slice(2)),
  ])
  console.log(`[proteus-code] starting: ${launch.command} ${launch.args.join(' ')}`)

  const child = spawn(launch.command, launch.args, {
    cwd: paths.repoRoot,
    env: launch.env,
    stdio: 'inherit',
  })

  const forward = (signal: NodeJS.Signals) => () => child.kill(signal)
  process.on('SIGINT', forward('SIGINT'))
  process.on('SIGTERM', forward('SIGTERM'))
  child.on('exit', (code) => process.exit(code ?? 0))
}

main().catch((error: unknown) => {
  console.error('[proteus-code]', error instanceof Error ? error.message : error)
  process.exit(1)
})
