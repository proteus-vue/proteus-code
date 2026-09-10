/**
 * proteus-code desktop main process.
 *
 * Boots the proteus-code Harness profile as a child process, waits for the Web
 * UI URL, and loads it in a native window. The child binds an OS-assigned
 * loopback port so several harnesses can coexist; the window only ever loads
 * that loopback origin.
 *
 * @module @proteus-code/desktop/main
 */

import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process'
import { app, BrowserWindow, dialog, shell } from 'electron'
import { ensureProfile, harnessLaunch, PROFILE_NAME, resolvePaths } from './profile.js'
import { forceDarkScheme, dumpSurfaces, readProbe } from './probe.js'

/** Matches the `dsh web: http://127.0.0.1:<port>/?token=…` readiness line. */
const READY_PATTERN = /(https?:\/\/\S+)/

/** How long to wait for the harness to print its URL before failing. */
const READY_TIMEOUT_MS = 120_000

let harness: ChildProcessWithoutNullStreams | undefined
let mainWindow: BrowserWindow | undefined

/** Resolve with the first URL the harness prints, or reject on timeout/exit. */
function waitForUrl(child: ChildProcessWithoutNullStreams): Promise<string> {
  return new Promise((resolve, reject) => {
    let buffer = ''
    const timer = setTimeout(() => {
      reject(new Error(`the harness did not report a URL within ${READY_TIMEOUT_MS}ms`))
    }, READY_TIMEOUT_MS)

    const onData = (chunk: Buffer) => {
      buffer += chunk.toString()
      const match = READY_PATTERN.exec(buffer)
      if (match?.[1]) {
        cleanup()
        resolve(match[1])
      }
    }
    const onExit = (code: number | null) => {
      cleanup()
      reject(new Error(`the harness exited (code ${code}) before reporting a URL\n${buffer}`))
    }
    const cleanup = () => {
      clearTimeout(timer)
      child.stdout.off('data', onData)
      child.stderr.off('data', onData)
      child.off('exit', onExit)
    }

    child.stdout.on('data', onData)
    child.stderr.on('data', onData)
    child.on('exit', onExit)
  })
}

/** Start the harness child and return its UI URL. */
async function startHarness(): Promise<string> {
  const paths = resolvePaths(app.getAppPath())
  app.setName('proteus-code')

  await ensureProfile(paths, (message) => console.log(message))

  const launch = harnessLaunch(paths, [
    '--profile',
    PROFILE_NAME,
    '--no-open',
    '--port',
    '0',
  ])
  harness = spawn(launch.command, launch.args, {
    cwd: paths.repoRoot,
    env: launch.env,
    stdio: ['pipe', 'pipe', 'pipe'],
  }) as ChildProcessWithoutNullStreams

  harness.on('error', (error) => {
    console.error('[proteus-code] failed to start the harness:', error)
  })
  return await waitForUrl(harness)
}

/** Build the application window. */
function createWindow(url: string): BrowserWindow {
  const window = new BrowserWindow({
    width: 1440,
    height: 940,
    minWidth: 720,
    minHeight: 560,
    title: 'proteus-code',
    backgroundColor: '#0b0f14',
    titleBarStyle: process.platform === 'darwin' ? 'hiddenInset' : 'default',
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      webSecurity: true,
    },
  })

  // The client shell rewrites document.title to the upstream product name; the
  // window title is ours, so keep it pinned to proteus-code.
  window.on('page-title-updated', (event) => {
    event.preventDefault()
    window.setTitle('proteus-code')
  })
  window.webContents.on('page-title-updated', (event) => event.preventDefault())

  // Keep navigation inside the harness origin; send everything else to the
  // user's browser instead of replacing the app window.
  const appOrigin = new URL(url).origin
  window.webContents.setWindowOpenHandler(({ url: target }) => {
    if (new URL(target).origin === appOrigin) return { action: 'allow' }
    void shell.openExternal(target)
    return { action: 'deny' }
  })
  window.webContents.on('will-navigate', (event, target) => {
    if (new URL(target).origin !== appOrigin) {
      event.preventDefault()
      void shell.openExternal(target)
    }
  })

  void window.loadURL(url)
  return window
}

/** Report a startup failure and quit. */
function fail(error: unknown): void {
  const message = error instanceof Error ? error.message : String(error)
  console.error('[proteus-code]', message)
  if (app.isReady()) {
    void dialog.showMessageBox({ type: 'error', title: 'proteus-code', message })
  }
  stopHarness()
  app.exit(1)
}

/**
 * Smoke mode: load the UI, wait for it to settle, capture the window to a PNG,
 * and exit. Used by `pnpm smoke` and CI to prove the rendered app works without
 * a human at the screen. Enabled by PROTEUS_CODE_SMOKE=<output.png>.
 */
async function runSmoke(window: BrowserWindow, outputPath: string): Promise<void> {
  const problems: string[] = []
  window.webContents.on('console-message', (_event, level, message) => {
    if (level >= 2) problems.push(`console: ${message}`)
  })
  window.webContents.on('did-fail-load', (_event, code, description, url) => {
    problems.push(`load-failed ${code} ${description} ${url}`)
  })

  const finish = async (code: number) => {
    stopHarness()
    app.exit(code)
  }
  try {
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('the UI did not finish loading in 60s')), 60_000)
      window.webContents.once('did-finish-load', () => {
        clearTimeout(timer)
        resolve()
      })
    })
    // Let the client hydrate and issue its first render before capturing.
    const settleMs = Number(process.env.PROTEUS_CODE_SMOKE_WAIT_MS ?? '20000')
    await new Promise((resolve) => setTimeout(resolve, settleMs))

    const state = (await window.webContents.executeJavaScript(
      `(() => ({
         title: document.title,
         hasRoot: Boolean(document.querySelector('#root, #app, body > div')),
         text: (document.body.innerText || '').slice(0, 400),
         html: document.body.innerHTML.length,
       }))()`,
    )) as { title: string; hasRoot: boolean; text: string; html: number }

    // Optional diagnostics: report computed theme tokens and control state, so
    // token-level restyling and composer gating can be checked numerically.
    // PROTEUS_CODE_DARK=1 forces the dark scheme first.
    if (process.env.PROTEUS_CODE_DARK === '1') {
      await forceDarkScheme(window)
    }
    if (process.env.PROTEUS_CODE_PROBE) {
      const probe = await readProbe(window)
      console.log(`[proteus-code] probe: ${JSON.stringify(probe, null, 2)}`)
    }
    if (process.env.PROTEUS_CODE_DUMP) {
      const dump = await dumpSurfaces(window)
      const { writeFileSync } = await import('node:fs')
      writeFileSync(process.env.PROTEUS_CODE_DUMP, `${JSON.stringify(dump, null, 2)}\n`)
      console.log(`[proteus-code] surfaces dumped → ${process.env.PROTEUS_CODE_DUMP}`)
    }
    if (process.env.PROTEUS_CODE_QUERY) {
      // Arbitrary read-only introspection: a script path whose last expression
      // is the value to report. Used to answer "what is this token here?" while
      // tuning, without keeping a bespoke probe for every question.
      const { readFileSync } = await import('node:fs')
      const script = readFileSync(process.env.PROTEUS_CODE_QUERY, 'utf8')
      const result = await window.webContents.executeJavaScript(script)
      console.log(`[proteus-code] query: ${JSON.stringify(result, null, 2)}`)
    }

    const image = await window.webContents.capturePage()
    const { writeFileSync, mkdirSync } = await import('node:fs')
    const path = await import('node:path')
    mkdirSync(path.dirname(outputPath), { recursive: true })
    writeFileSync(outputPath, image.toPNG())
    writeFileSync(`${outputPath}.txt`, `${JSON.stringify({ ...state, problems }, null, 2)}\n`)

    console.log(
      `[proteus-code] smoke: captured ${outputPath} title=${JSON.stringify(state.title)} ` +
        `htmlBytes=${state.html} problems=${problems.length}`,
    )
    console.log(`[proteus-code] smoke body: ${JSON.stringify(state.text)}`)
    for (const problem of problems.slice(0, 10)) console.log(`[proteus-code] smoke ! ${problem}`)
    await finish(state.hasRoot ? 0 : 1)
  } catch (error) {
    console.error('[proteus-code] smoke failed:', error instanceof Error ? error.message : error)
    for (const problem of problems.slice(0, 10)) console.error('[proteus-code] smoke !', problem)
    await finish(1)
  }
}

/** Terminate the harness child if it is still running. */
function stopHarness(): void {
  if (harness && !harness.killed) {
    harness.kill('SIGTERM')
    harness = undefined
  }
}

app.whenReady().then(async () => {
  try {
    const url = await startHarness()
    console.log(`[proteus-code] UI ready at ${url}`)
    mainWindow = createWindow(url)
    mainWindow.on('closed', () => {
      mainWindow = undefined
    })
    const smokeOutput = process.env.PROTEUS_CODE_SMOKE
    if (smokeOutput) await runSmoke(mainWindow, smokeOutput)
  } catch (error) {
    fail(error)
  }
})

app.on('window-all-closed', () => {
  stopHarness()
  if (process.platform !== 'darwin') app.quit()
})

app.on('activate', () => {
  // The harness child owns the session; a closed window means a relaunch.
  if (BrowserWindow.getAllWindows().length === 0) {
    app.relaunch()
    app.exit(0)
  }
})

app.on('before-quit', stopHarness)
process.on('exit', stopHarness)
