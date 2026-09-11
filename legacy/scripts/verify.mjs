/**
 * End-to-end verification for proteus-code.
 *
 * Runs the build, the unit tests, and a real harness boot against a throwaway
 * Harness home, asserting that the composed profile serves the UI. Exits nonzero
 * on the first failure.
 */

import { spawn, spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const pnpm = process.env.PNPM_BIN ?? 'pnpm'

function step(label) {
  console.log(`\n=== ${label} ===`)
}

function run(label, command, args, options = {}) {
  step(label)
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit', ...options })
  if (result.status !== 0) {
    console.error(`\n✗ ${label} failed (exit ${result.status})`)
    process.exit(result.status ?? 1)
  }
  console.log(`✓ ${label}`)
}

run('build the bundle', pnpm, ['run', 'build:bundle'])
run('unit tests', pnpm, ['run', 'test'])
run('typecheck', pnpm, ['run', 'typecheck'])

step('boot the proteus-code profile with the bundle installed')
const home = mkdtempSync(path.join(tmpdir(), 'proteus-code-verify-'))
const port = 3300 + Math.floor(Math.random() * 400)
const dshEntry = path.join(root, 'node_modules/@deepseek-ai/dsh/lib/bin.js')
if (!existsSync(dshEntry)) {
  console.error(`✗ DSH entry missing at ${dshEntry}; run pnpm install`)
  process.exit(1)
}

const env = { ...process.env, DSH_HOME: home }
const profileDir = path.join(home, 'profiles', 'proteus-code')
const bundleDir = path.join(root, 'packages/proteus-code-bundle')

// Prepare the profile exactly as the desktop app does, then install the bundle
// the same way. A plugin that fails to import aborts the boot, so a served UI
// here is also proof that the host face loaded.
mkdirSync(path.join(profileDir), { recursive: true })
writeFileSync(
  path.join(profileDir, 'package.json'),
  `${JSON.stringify(
    {
      name: 'dsh-profile-proteus-code',
      private: true,
      dependencies: {},
      dsh: {
        profile: {
          bundles: ['@deepseek-ai/dsh-base', '@deepseek-ai/dsh-web-app', '@proteus-code/dsh-proteus-code'],
          patchReload: 'live',
        },
      },
    },
    null,
    2,
  )}\n`,
)
writeFileSync(path.join(profileDir, 'pnpm-workspace.yaml'), 'packages:\n  - .\n\nnodeLinker: hoisted\nautoInstallPeers: false\n')

const install = spawnSync(
  process.execPath,
  [dshEntry, 'plugin', '--profile', 'proteus-code', 'add', '-w', `file:${bundleDir}`],
  { cwd: root, env, stdio: 'inherit' },
)
if (install.status !== 0) {
  console.error('✗ installing the Proteus bundle failed')
  process.exit(install.status ?? 1)
}

const child = spawn(
  process.execPath,
  [dshEntry, '--profile', 'proteus-code', '--no-open', '--port', String(port)],
  { cwd: root, env, stdio: ['ignore', 'pipe', 'pipe'] },
)

let output = ''
child.stdout.on('data', (chunk) => (output += chunk))
child.stderr.on('data', (chunk) => (output += chunk))

const deadline = Date.now() + 120_000
let ready = false
while (Date.now() < deadline) {
  if (/https?:\/\/127\.0\.0\.1/.test(output)) {
    ready = true
    break
  }
  await new Promise((resolve) => setTimeout(resolve, 1_000))
}

let exitCode = 0
if (!ready) {
  console.error('✗ the harness never reported a URL')
  console.error(output.slice(-2000))
  exitCode = 1
} else {
  const response = await fetch(`http://127.0.0.1:${port}/`).catch(() => undefined)
  if (response && (response.status === 401 || response.status === 403 || response.status === 200)) {
    console.log(`✓ harness serves the UI (HTTP ${response.status}, gated as expected)`)
  } else {
    console.error(`✗ unexpected HTTP response from the harness: ${response?.status ?? 'none'}`)
    exitCode = 1
  }
}

child.kill('SIGTERM')
rmSync(home, { recursive: true, force: true })

if (exitCode === 0) console.log('\n✓ verify passed')
process.exit(exitCode)
