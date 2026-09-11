/**
 * Browser face of the proteus-code bundle.
 *
 * A `dsh.client` dual-face package: this module runs in the Web renderer and
 * contributes the product's identity to three surfaces:
 *
 *   1. the **theme** — a full `--dsw-alias-*` token layer, so the entire UI
 *      adopts the Proteus palette;
 *   2. the **brand slots** — the sidebar and welcome-hero marks and wordmark;
 *   3. the **document title** — pinned against the host's own writes.
 *
 * It is built into the `window.__ModuleLoader__.load({...})` wrapper the client
 * module system expects (see scripts/build-bundle.mjs).
 *
 * @module @proteus-code/dsh-proteus-code/client
 */

import type { ComponentType } from 'react'
import { jsx } from 'react/jsx-runtime'
import { installCopyOverrides } from './copy-overrides.ts'
import { BRAND, PROTEUS_TOKENS, type TokenOverrideTable } from './palette.ts'
import { createWorkspacePicker, type PickerServices, type WorkspaceRow } from './workspace-picker.tsx'

/** Layer identity for the theme override; one layer per source. */
const THEME_SOURCE = '@proteus-code/dsh-proteus-code'

/** Registration context provided to the client plugin. */
interface ClientSlotContext {
  slots: {
    inject(slot: string, callback: () => unknown): void
    /**
     * Register into a slot. `priority` decides single-slot occupancy: the LOWEST
     * value renders, so a lower priority than the incumbent shadows it without
     * needing that incumbent to be disabled from the host patch.
     */
    register(
      declaration: { name: string; priority?: number },
      component: ComponentType<Record<string, unknown>>,
    ): () => void
  }
}

/** Nested registration context for the theme service. */
interface ClientRootContext extends ClientSlotContext {
  inject(
    deps: string[],
    callback: (scope: Record<string, unknown>) => void,
  ): unknown
}

/** The theme service surface this module uses. */
interface ThemeSeam {
  overrideTokens(source: string, tokens: TokenOverrideTable): () => void
}

/**
 * Host-supplied options, injected into the boot HTML.
 *
 * The client face receives no plugin config from the harness, so the host face
 * forwards the few runtime choices the browser needs. This is also the kill
 * switch for the workspace-picker takeover: replacing a `single` slot is the one
 * change here that can remove existing UI, so it must be turnable off without a
 * rebuild.
 */
interface ProteusClientOptions {
  readonly workspacePicker?: boolean
  /** Forwarded from the host: the directory an unprojected session runs in. */
  readonly defaultCwd?: string
}

function clientOptions(): ProteusClientOptions {
  const raw = (globalThis as { __PROTEUS_CODE_OPTS__?: unknown }).__PROTEUS_CODE_OPTS__
  return typeof raw === 'object' && raw !== null ? (raw as ProteusClientOptions) : {}
}

/**
 * Unwrap a remote result envelope, or throw the transport error.
 *
 * Accepts both the enveloped form (`{ ok, value | error }`) and a bare value,
 * because the transport is not the only possible caller here.
 */
function unwrap<T>(result: unknown): T {
  const envelope = result as { ok?: boolean; value?: T; error?: unknown } | undefined
  if (envelope && envelope.ok === false) {
    const error = envelope.error as { message?: string } | string | undefined
    const message = typeof error === 'string' ? error : error?.message
    throw new Error(message ?? 'the harness rejected the request')
  }
  return (envelope && envelope.ok === true ? envelope.value : result) as T
}


/** Normalize whatever the workspace service returns into rows. */
export function toWorkspaceRows(snapshot: unknown): WorkspaceRow[] {
  const list = Array.isArray(snapshot)
    ? snapshot
    : ((snapshot as { items?: unknown[] } | undefined)?.items ?? [])
  return list
    .map((entry) => {
      const w = entry as Record<string, unknown>
      const id = (w.workspaceId ?? w.id) as string | undefined
      if (!id) return undefined
      const title = (w.title ?? w.name ?? w.path ?? id) as string
      return { workspaceId: id, title }
    })
    .filter((row): row is WorkspaceRow => row !== undefined)
}

/** Build the picker's service adapter from the client services. */
function pickerServices(scope: Record<string, unknown>): PickerServices {
  const workspaces = scope.workspaces as {
    list(): Promise<unknown>
    create(input: { path: string }): Promise<unknown>
  }
  const picker = (scope['remote.directoryPicker'] ?? scope.remote) as {
    pick(): Promise<unknown>
  }

  return {
    list: async () => toWorkspaceRows(await workspaces.list()),
    pickDirectory: async () => unwrap<{ path: string }>(await picker.pick()).path,
    createWorkspace: async (path) => toWorkspaceRows([unwrap(await workspaces.create({ path }))])[0]!,
    /**
     * Adopt the harness's own working directory as the current project.
     *
     * A workspace-less session is not a usable state in this UI: the composer is
     * disabled until the hero's workspace chip has a label, and that label comes
     * from either a workspace or an attached directory. The harness's own
     * "new session" control reflects this — with no workspace it only clears the
     * selection and returns to the picker.
     *
     * So "work without picking a project" is served the way the product supports:
     * register the working directory as a project and open it. The user picks one
     * item and is composing; nothing is set up by hand. `create` is idempotent, so
     * repeated use reuses the same entry rather than duplicating it.
     */
    adoptDefaultProject: async () => {
      const cwd = clientOptions().defaultCwd
      if (!cwd) {
        throw new Error(
          'no default directory was provided by the host, so there is nowhere to work',
        )
      }
      const workspace = toWorkspaceRows([unwrap(await workspaces.create({ path: cwd }))])[0]!
      return workspace.workspaceId
    },
  }
}

/** The protrusive "P" mark: a rounded badge with a transform silhouette. */
function ProteusBrandMark({ size = 24 }: { size?: number }) {
  return jsx('svg', {
    width: size,
    height: size,
    viewBox: '0 0 32 32',
    role: 'img',
    'aria-label': 'proteus code',
    children: [
      jsx('defs', {
        children: jsx('linearGradient', {
          id: 'proteus-code-mark',
          x1: '0',
          y1: '0',
          x2: '1',
          y2: '1',
          children: [
            jsx('stop', { offset: '0%', stopColor: BRAND.cyan }),
            jsx('stop', { offset: '55%', stopColor: BRAND.primaryLight }),
            jsx('stop', { offset: '100%', stopColor: BRAND.violet }),
          ],
        }),
      }),
      jsx('rect', {
        x: 1,
        y: 1,
        width: 30,
        height: 30,
        rx: 9,
        fill: 'url(#proteus-code-mark)',
      }),
      // A prow-like glyph: one form, many headings.
      jsx('path', {
        d: 'M11 23V9h6.2c3.1 0 5.3 1.9 5.3 4.7 0 2.9-2.2 4.8-5.3 4.8H14.6V23H11zm3.6-7.6h2.3c1.3 0 2.1-.7 2.1-1.7s-.8-1.7-2.1-1.7h-2.3v3.4z',
        fill: '#ffffff',
      }),
    ],
  })
}

/** The proteus-code wordmark shown beside the mark. */
function ProteusBrandName() {
  return jsx('span', {
    style: {
      display: 'inline-flex',
      alignItems: 'baseline',
      gap: '6px',
      fontWeight: 600,
      letterSpacing: '-0.01em',
      whiteSpace: 'nowrap',
    },
    children: [
      jsx('span', {
        style: {
          background: `linear-gradient(90deg, ${BRAND.cyan}, ${BRAND.primaryLight} 55%, ${BRAND.violet})`,
          WebkitBackgroundClip: 'text',
          backgroundClip: 'text',
          color: 'transparent',
        },
        children: 'proteus',
      }),
      jsx('span', {
        style: {
          fontSize: '0.68em',
          fontWeight: 700,
          letterSpacing: '0.09em',
          textTransform: 'uppercase',
          padding: '2px 7px',
          borderRadius: '999px',
          background: 'rgba(99,102,241,0.14)',
          border: '1px solid rgba(99,102,241,0.32)',
          color: 'inherit',
        },
        children: 'code',
      }),
    ],
  })
}

/**
 * Keep the document title fixed at the product name.
 *
 * The layout plugin writes `document.title` from a hardcoded build-time product
 * name on every session/panel change, so a one-shot assignment is overwritten.
 * Redefining the accessor on the document instance shadows the prototype
 * definition and ignores those writes; a MutationObserver backstop covers
 * environments where the property cannot be redefined.
 */
function ownDocumentTitle(title: string): void {
  if (typeof document === 'undefined') return
  document.title = title
  try {
    Object.defineProperty(document, 'title', {
      configurable: true,
      get: () => title,
      set: () => undefined,
    })
    return
  } catch {
    // Fall through to the observer below.
  }
  const enforce = () => {
    if (document.title !== title) document.title = title
  }
  new MutationObserver(enforce).observe(document.head ?? document.documentElement, {
    childList: true,
    subtree: true,
    characterData: true,
  })
}

/** Required service: the UI slot registry. */
export const inject = ['slots']

/**
 * Contribute the Proteus identity: theme tokens, brand slots, and the document
 * title. The theme registration waits on the `theme` service, so load order
 * does not matter.
 */
export function apply(ctx: ClientRootContext): void {
  ownDocumentTitle('proteus code')

  // Strings owned by namespaces this bundle cannot re-register; see the module.
  installCopyOverrides(document)

  ctx.inject(['theme'], (scope) => {
    ;(scope.theme as ThemeSeam).overrideTokens(THEME_SOURCE, PROTEUS_TOKENS)
  })

  // The ZCode-style picker, including "work without a project". This owns a
  // `single` slot, so it is opted out of via injected options rather than forced.
  if (clientOptions().workspacePicker !== false) {
    ctx.inject(['workspaces', 'remote.directoryPicker'], (scope) => {
      const services = pickerServices(scope)
      ctx.slots.inject('conversation.hero.workspace', () => {
        ctx.slots.register(
          // Priority -1 shadows the stock picker (priority 0) — the framework's
          // documented single-slot arbitration: the lowest value renders.
          { name: 'conversation.hero.workspace', priority: -1 },
          createWorkspacePicker(services) as unknown as ComponentType<Record<string, unknown>>,
        )
      })
    })
  }

  ctx.slots.inject('sidebar.brand.mark', () =>
    ctx.slots.inject('sidebar.brand.name', function* () {
      yield ctx.slots.register({ name: 'sidebar.brand.mark' }, ProteusBrandMark)
      yield ctx.slots.register({ name: 'sidebar.brand.name' }, ProteusBrandName)
    }),
  )
  ctx.slots.inject('conversation.hero.brand.mark', () =>
    ctx.slots.register({ name: 'conversation.hero.brand.mark' }, ProteusBrandMark),
  )
}
