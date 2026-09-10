/**
 * Build the Electron main process.
 *
 * Bundles `src/main.ts` (and its local modules) to `dist/main.js` as CommonJS,
 * which Electron's main process loads reliably, while leaving `electron` itself
 * external.
 */

import { build } from 'esbuild'
import { mkdirSync, rmSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const pkgDir = path.resolve(here, '..')
const outDir = path.join(pkgDir, 'dist')

rmSync(outDir, { recursive: true, force: true })
mkdirSync(outDir, { recursive: true })

await build({
  entryPoints: [
    path.join(pkgDir, 'src/main.ts'),
    path.join(pkgDir, 'src/cli.ts'),
  ],
  outdir: outDir,
  bundle: true,
  // The package is `type: module`, and Electron 28+ loads an ESM main process,
  // so both entries stay ESM and share one resolution model.
  format: 'esm',
  platform: 'node',
  target: 'node22',
  external: ['electron'],
  sourcemap: false,
  logLevel: 'info',
})

console.log('[proteus-code] desktop built → apps/desktop/dist/{main.js,cli.js}')
