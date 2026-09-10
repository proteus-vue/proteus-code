/**
 * Build the proteus-code DSH bundle.
 *
 * Produces two faces of the same package:
 *   - `dist/index.js`  — the host plugin, bundled ESM with `@deepseek-ai/*` left
 *                        external so it shares the harness's service singletons.
 *   - `dist/client.js` — the browser plugin, bundled CommonJS with `react` and
 *                        `@deepseek-ai/*` external, then wrapped in the
 *                        `window.__ModuleLoader__.load({ id, factory })` envelope
 *                        the DSH client module system expects.
 *
 * Types are emitted with `tsc`.
 */

import { build } from 'esbuild'
import { execFileSync } from 'node:child_process'
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const root = path.resolve(here, '..')
const pkgDir = path.join(root, 'packages/proteus-code-bundle')
const outDir = path.join(pkgDir, 'dist')
const pkgName = '@proteus-code/dsh-proteus-code'

rmSync(outDir, { recursive: true, force: true })
mkdirSync(outDir, { recursive: true })

// ── host face ────────────────────────────────────────────────────────────────

await build({
  entryPoints: [path.join(pkgDir, 'src/index.ts')],
  outfile: path.join(outDir, 'index.js'),
  bundle: true,
  format: 'esm',
  platform: 'node',
  target: 'node22',
  sourcemap: false,
  // Harness packages must stay external: the running dsh installation provides
  // them, and bundling them would duplicate Cordis service instances.
  external: ['@deepseek-ai/*'],
  logLevel: 'warning',
})

// ── browser face ─────────────────────────────────────────────────────────────

const clientResult = await build({
  entryPoints: [path.join(pkgDir, 'src/client/index.tsx')],
  bundle: true,
  write: false,
  format: 'cjs',
  platform: 'browser',
  target: 'es2022',
  jsx: 'automatic',
  sourcemap: false,
  // The renderer supplies these; they must resolve through its module loader.
  external: ['react', 'react/jsx-runtime', 'react-dom', '@deepseek-ai/*'],
  logLevel: 'warning',
})

const clientBody = clientResult.outputFiles[0]?.text
if (!clientBody) throw new Error('client bundle produced no output')

const wrapped = `window.__ModuleLoader__.load({
	id: ${JSON.stringify(pkgName)},
	factory: (require) => {
		var module = { exports: {} };
		var exports = module.exports;
${clientBody
  .split('\n')
  .map((line) => (line.length > 0 ? `\t\t${line}` : line))
  .join('\n')}
		return module.exports;
	}
});
`
writeFileSync(path.join(outDir, 'client.js'), wrapped, 'utf8')

// ── types ────────────────────────────────────────────────────────────────────

const tsc = path.join(root, 'node_modules/typescript/bin/tsc')
execFileSync(process.execPath, [tsc, '--emitDeclarationOnly', '-p', path.join(pkgDir, 'tsconfig.json')], {
  stdio: 'inherit',
  cwd: pkgDir,
})

const hostBytes = readFileSync(path.join(outDir, 'index.js')).length
const clientBytes = readFileSync(path.join(outDir, 'client.js')).length
console.log(
  `[proteus-code] bundle built → dist/index.js (${hostBytes} B), dist/client.js (${clientBytes} B)`,
)
