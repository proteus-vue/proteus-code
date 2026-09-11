/**
 * The ZCode-style workspace picker for the new-session hero.
 *
 * ## Why this replaces the stock picker rather than adding to it
 *
 * `conversation.hero.workspace` is a `kind: "single"` slot, so a contribution
 * either is the dropdown or is not rendered at all. Adding an option therefore
 * means owning the dropdown — and owning it means re-providing every capability
 * the stock one has, or the change is a regression, not a feature.
 *
 * What the stock dropdown offers, and how each is preserved here:
 *
 * | Capability | Preserved by |
 * |---|---|
 * | Recent workspaces, current one marked | read from `ctx.workspaces` |
 * | Switching to a workspace | delegating to `onPick(workspaceId)` — the conversation plugin keeps ownership of navigation |
 * | Choosing a folder | `ctx.remote.directoryPicker.pick()` then `ctx.workspaces.create({ path })` |
 * | **Working without a project** | `ctx.sessions.create()` with no cwd, then open |
 *
 * The last row is the addition. It works because the host's `session.create`
 * documents a `defaultCwd` fallback for a request that names neither a workspace
 * nor a cwd — the harness's own definition of "run here, ungrouped". Reaching it
 * needs no new backend behavior, and the resulting session behaves like any
 * other: same tools, same sandbox policy, same persistence. It simply has no
 * project grouping.
 *
 * Dropped deliberately: the in-app directory browser and its create-directory
 * flow. The native OS picker is the better desktop affordance, and reimplementing
 * a browser that desktop users never see would be surface without value.
 *
 * @module @proteus-code/dsh-proteus-code/client/workspace-picker
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { jsx } from 'react/jsx-runtime'

/** One workspace row as the picker needs it. */
export interface WorkspaceRow {
  readonly workspaceId: string
  readonly title: string
}

/** The service surface this picker uses, resolved lazily at render time. */
export interface PickerServices {
  list(): Promise<readonly WorkspaceRow[]>
  pickDirectory(): Promise<string>
  createWorkspace(path: string): Promise<WorkspaceRow>
  /**
   * Register (or reuse) the harness working directory as a project and return
   * its id. This is how "no project to pick" is served: the UI needs a workspace
   * for the composer to be usable, so the directory is adopted automatically and
   * the caller then selects it through the normal path.
   */
  adoptDefaultProject(): Promise<string>
}

/** Props the conversation plugin passes into the slot. */
export interface WorkspacePickerProps {
  open: boolean
  anchorRef?: { current: HTMLElement | null }
  selectedId?: string
  onPick: (workspaceId: string) => void
  onClose: () => void
}

/** Copy for both UI languages, chosen from the document language. */
export function pickerLabels(zh: boolean) {
  return {
    search: zh ? '搜索工作区' : 'Search workspaces',
    openFolder: zh ? '打开文件夹' : 'Open folder',
    noProject: zh ? '不在项目中工作' : 'Work without a project',
    empty: zh ? '暂无工作区' : 'No workspaces yet',
    failed: zh ? '操作失败' : 'That did not work',
  }
}

/** Filter workspaces by a case-insensitive title substring. */
export function filterWorkspaces(
  rows: readonly WorkspaceRow[],
  query: string,
): readonly WorkspaceRow[] {
  const needle = query.trim().toLowerCase()
  if (needle.length === 0) return rows
  return rows.filter((row) => row.title.toLowerCase().includes(needle))
}

/** Whether the current document language is Chinese. */
function prefersChinese(): boolean {
  if (typeof document === 'undefined') return false
  return (document.documentElement.lang || navigator.language || '').toLowerCase().startsWith('zh')
}

/** Inline styles, referencing the host's own tokens so the theme applies. */
const S = {
  menu: {
    position: 'fixed',
    zIndex: '2147483000',
    minWidth: '300px',
    maxHeight: '60vh',
    overflowY: 'auto',
    padding: '6px',
    borderRadius: '16px',
    background: 'var(--dsw-alias-bg-layer-2)',
    color: 'var(--dsw-alias-label-primary)',
    border: '1px solid var(--dsw-alias-border-l2)',
    boxShadow: '0 24px 60px -16px rgba(11,16,32,0.34), 0 2px 6px rgba(11,16,32,0.14)',
    backdropFilter: 'blur(24px) saturate(180%)',
    WebkitBackdropFilter: 'blur(24px) saturate(180%)',
    fontSize: '14px',
    lineHeight: '22px',
  } as const,
  search: {
    width: '100%',
    boxSizing: 'border-box',
    padding: '9px 12px',
    marginBottom: '4px',
    borderRadius: '10px',
    border: '1px solid var(--dsw-alias-border-l2)',
    background: 'var(--dsw-alias-bg-module-platform)',
    color: 'var(--dsw-alias-label-primary)',
    outline: 'none',
    fontSize: '14px',
    fontFamily: 'inherit',
  } as const,
  /**
   * A menu row's background by interaction state.
   *
   * Three distinct states, matching what the host's own menus show:
   *   - `active`   — the current cursor position (mouse hover OR arrow-key
   *                  focus). This is the "landing point" the eye needs.
   *   - `selected` — the workspace the session currently belongs to.
   *   - `idle`     — everything else.
   *
   * Inline styles cannot express `:hover`, so hover is tracked in React state
   * and shares one cursor with the keyboard. `active` wins over `selected`: while
   * the pointer is moving, the highlight must follow it.
   */
  row: (state: 'idle' | 'selected' | 'active') =>
    ({
      display: 'flex',
      alignItems: 'center',
      gap: '10px',
      width: '100%',
      padding: '9px 12px',
      borderRadius: '10px',
      border: 'none',
      background:
        state === 'active'
          ? 'var(--dsw-alias-interactive-bg-hover)'
          : state === 'selected'
            ? 'var(--dsw-alias-interactive-bg-active)'
            : 'transparent',
      color: 'var(--dsw-alias-label-primary)',
      cursor: 'pointer',
      textAlign: 'left',
      font: 'inherit',
      outline: 'none',
      transition: 'background-color 120ms ease',
    }) as const,
  divider: {
    height: '1px',
    margin: '6px 8px',
    background: 'var(--dsw-alias-border-l2)',
  } as const,
  list: {
    display: 'flex',
    flexDirection: 'column',
    gap: '1px',
  } as const,
  empty: {
    padding: '10px 12px',
    color: 'var(--dsw-alias-label-tertiary)',
  } as const,
  check: { marginLeft: 'auto', color: 'var(--dsw-alias-brand-primary)' } as const,
  error: {
    margin: '6px 8px 2px',
    padding: '8px 10px',
    borderRadius: '8px',
    color: 'var(--dsw-alias-state-error-primary)',
    background: 'var(--dsw-alias-state-error-secondary)',
    fontSize: '13px',
  } as const,
} as const

/** A folder glyph, drawn inline so no asset is loaded. */
function FolderIcon() {
  return jsx('svg', {
    width: 16,
    height: 16,
    viewBox: '0 0 16 16',
    fill: 'none',
    stroke: 'currentColor',
    strokeWidth: 1.4,
    children: jsx('path', {
      d: 'M1.8 4.2a1 1 0 0 1 1-1h3l1.4 1.6h6a1 1 0 0 1 1 1v5.4a1 1 0 0 1-1 1H2.8a1 1 0 0 1-1-1V4.2Z',
    }),
  })
}

/** A speech glyph for the no-project entry. */
function ChatIcon() {
  return jsx('svg', {
    width: 16,
    height: 16,
    viewBox: '0 0 16 16',
    fill: 'none',
    stroke: 'currentColor',
    strokeWidth: 1.4,
    children: jsx('path', {
      d: 'M2 7.4c0-2.5 2.3-4.4 6-4.4s6 1.9 6 4.4-2.3 4.4-6 4.4c-.6 0-1.2-.1-1.7-.2L3.4 13l.5-2.3A4.6 4.6 0 0 1 2 7.4Z',
    }),
  })
}

/**
 * Advance a menu cursor for one key press.
 *
 * Extracted as a pure function so the navigation rules (wrap-around, Home/End)
 * are testable without a DOM event pipeline. Returns the next index, or
 * `undefined` when the key is not a navigation key.
 *
 * An empty menu always yields `-1` (no cursor). Down from nothing lands on the
 * first item and Up lands on the last, matching how native menus behave.
 */
export function nextCursor(
  current: number,
  key: string,
  length: number,
): number | undefined {
  if (length <= 0) return key === 'Home' || key === 'End' || key.startsWith('Arrow') ? -1 : undefined
  switch (key) {
    case 'ArrowDown':
      return current < 0 ? 0 : (current + 1) % length
    case 'ArrowUp':
      return current < 0 ? length - 1 : (current - 1 + length) % length
    case 'Home':
      return 0
    case 'End':
      return length - 1
    default:
      return undefined
  }
}

/**
 * Build the picker component bound to its services.
 *
 * The component keeps its own list state instead of the framework's store
 * binding: it only needs a refresh-per-open, and owning that keeps the
 * contribution self-contained.
 */
export function createWorkspacePicker(services: PickerServices) {
  return function WorkspacePicker(props: WorkspacePickerProps) {
    const { open, anchorRef, selectedId, onPick, onClose } = props
    const zh = prefersChinese()
    const t = pickerLabels(zh)

    const [rows, setRows] = useState<readonly WorkspaceRow[]>([])
    const [query, setQuery] = useState('')
    const [error, setError] = useState<string | undefined>(undefined)
    const [pos, setPos] = useState<{ left: number; top: number } | undefined>(undefined)
    /** Cursor position shared by hover and arrow keys; -1 means "nothing yet". */
    const [cursor, setCursor] = useState(-1)
    const menuRef = useRef<HTMLElement | null>(null)
    const searchRef = useRef<HTMLInputElement | null>(null)

    const refresh = useCallback(() => {
      services
        .list()
        .then(setRows)
        .catch(() => setRows([]))
    }, [services])

    // Anchor the menu to the chip on open, flipping above when it would overflow.
    // With no anchor element the menu must still open — falling back to a
    // centred-ish position beats silently rendering nothing.
    useEffect(() => {
      if (!open) return
      setQuery('')
      setError(undefined)
      refresh()
      const el = anchorRef?.current
      const estimated = Math.min(window.innerHeight * 0.6, 420)
      if (!el) {
        setPos({ left: Math.round(window.innerWidth / 2 - 160), top: 120 })
        return
      }
      const r = el.getBoundingClientRect()
      const below = r.bottom + 8
      const top = below + estimated > window.innerHeight ? Math.max(8, r.top - estimated - 8) : below
      setPos({ left: Math.max(8, r.left), top })
    }, [open, anchorRef, refresh])

    // Dismiss on outside click or Escape, the way any menu should.
    useEffect(() => {
      if (!open) return
      const onDown = (event: MouseEvent) => {
        const target = event.target as Node
        if (menuRef.current?.contains(target)) return
        if (anchorRef?.current?.contains(target)) return
        onClose()
      }
      const onKey = (event: KeyboardEvent) => {
        if (event.key === 'Escape') onClose()
      }
      // Deferred so the click that opened the menu does not immediately close it.
      const id = window.setTimeout(() => document.addEventListener('mousedown', onDown), 0)
      document.addEventListener('keydown', onKey)
      return () => {
        window.clearTimeout(id)
        document.removeEventListener('mousedown', onDown)
        document.removeEventListener('keydown', onKey)
      }
    }, [open, anchorRef, onClose])

    const visible = useMemo(() => filterWorkspaces(rows, query), [rows, query])

    /**
     * Run an action, reporting failure where the user can see it.
     *
     * The menu closes only on SUCCESS. Closing first would hide the error: the
     * failure is rendered inside the menu, so dismissing it on failure turns a
     * diagnosable problem into a silent no-op.
     */
    const run = useCallback(
      async (action: () => Promise<void>) => {
        setError(undefined)
        try {
          await action()
          onClose()
        } catch (reason) {
          setError(reason instanceof Error ? reason.message : t.failed)
        }
      },
      [onClose, t.failed],
    )

    const chooseFolder = () =>
      run(async () => {
        const path = await services.pickDirectory()
        const workspace = await services.createWorkspace(path)
        onPick(workspace.workspaceId)
      })

    /**
     * "Work without a project": adopt the harness working directory, then select
     * it through the same `onPick` every other row uses. The host owns the
     * navigation (it opens a session in the workspace), so this stays on the
     * proven path rather than reimplementing session creation.
     */
    const chooseNoProject = () =>
      run(async () => {
        const workspaceId = await services.adoptDefaultProject()
        onPick(workspaceId)
      })

    /**
     * One flat, ordered list of everything selectable.
     *
     * Keyboard traversal and hover share this order, so `cursor` is a single
     * index into it rather than per-section state. The divider is presentational
     * and deliberately not an item.
     */
    const items = useMemo(
      () => [
        ...visible.map((row) => ({ kind: 'workspace' as const, id: row.workspaceId, title: row.title })),
        { kind: 'folder' as const, id: '__folder__', title: t.openFolder },
        { kind: 'no-project' as const, id: '__no-project__', title: t.noProject },
      ],
      [visible, t.openFolder, t.noProject],
    )

    const activate = useCallback(
      (index: number) => {
        const item = items[index]
        if (!item) return
        // Selection is immediate; the two actions report their own outcome and
        // close on success only, so a failure stays visible.
        if (item.kind === 'workspace') {
          onClose()
          onPick(item.id)
        } else if (item.kind === 'folder') {
          void chooseFolder()
        } else {
          void chooseNoProject()
        }
      },
      // chooseFolder/chooseNoProject close over `services` and `t`, stable per open.
      // eslint-disable-next-line react-hooks/exhaustive-deps
      [items, onClose, onPick],
    )

    // Reset the cursor whenever the result set changes shape.
    useEffect(() => {
      setCursor(items.length > 0 ? 0 : -1)
    }, [items.length])

    // Arrow keys, Home/End, Enter — the keyboard half of the landing-point feedback.
    const onKeyDown = (event: { key: string; preventDefault: () => void }) => {
      const next = nextCursor(cursor, event.key, items.length)
      if (next !== undefined) {
        event.preventDefault()
        setCursor(next)
        return
      }
      if (event.key === 'Enter') {
        event.preventDefault()
        activate(cursor)
      }
    }

    // Focus the search field on open, so typing filters without a click.
    useEffect(() => {
      if (open && pos) searchRef.current?.focus()
    }, [open, pos])

    /** One rendered row, with its interaction state resolved. */
    const renderRow = (index: number, item: (typeof items)[number], leading: unknown, trailing?: unknown) =>
      jsx(
        'button',
        {
          key: item.id,
          type: 'button',
          role: 'menuitem',
          'aria-current': item.kind === 'workspace' && item.id === selectedId ? 'true' : undefined,
          style: S.row(index === cursor ? 'active' : item.kind === 'workspace' && item.id === selectedId ? 'selected' : 'idle'),
          // Hover moves the same cursor the keyboard uses. `onMouseEnter` alone
          // is enough (it fires per row entry); adding `onMouseMove` would
          // re-fire continuously while the pointer rests on a row.
          onMouseEnter: () => setCursor(index),
          onClick: () => activate(index),
          children: [
            jsx('span', { key: 'i', style: { display: 'flex', flex: 'none' }, children: leading }),
            jsx('span', { key: 't', style: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }, children: item.title }),
            trailing ?? null,
          ],
        },
        item.id,
      )

    if (!open || !pos) return null

    const workspaceCount = visible.length

    return jsx('div', {
      ref: menuRef,
      style: { ...S.menu, left: `${pos.left}px`, top: `${pos.top}px` },
      role: 'menu',
      'data-proteus-code': 'workspace-picker',
      onKeyDown,
      children: [
        jsx('input', {
          key: 'search',
          ref: searchRef,
          style: S.search,
          placeholder: t.search,
          value: query,
          'aria-label': t.search,
          onKeyDown,
          onChange: (e: { target: { value: string } }) => setQuery(e.target.value),
        }),
        workspaceCount === 0
          ? jsx('div', { key: 'empty', style: S.empty, children: t.empty })
          : jsx('div', {
              key: 'list',
              style: S.list,
              role: 'group',
              children: visible.map((row, i) =>
                renderRow(
                  i,
                  { kind: 'workspace', id: row.workspaceId, title: row.title },
                  FolderIcon(),
                  row.workspaceId === selectedId
                    ? jsx('span', { key: 'c', style: S.check, children: '✓' })
                    : null,
                ),
              ),
            }),
        jsx('div', { key: 'd', style: S.divider }),
        renderRow(workspaceCount, { kind: 'folder', id: '__folder__', title: t.openFolder }, FolderIcon()),
        renderRow(
          workspaceCount + 1,
          { kind: 'no-project', id: '__no-project__', title: t.noProject },
          ChatIcon(),
        ),
        error ? jsx('div', { key: 'err', style: S.error, children: error }) : null,
      ],
    })
  }
}
