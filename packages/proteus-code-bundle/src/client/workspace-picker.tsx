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
  /** Create an ungrouped session and return its id. */
  createUngroupedSession(): Promise<string>
  openSession(sessionId: string): void
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
  row: (selected: boolean) =>
    ({
      display: 'flex',
      alignItems: 'center',
      gap: '10px',
      width: '100%',
      padding: '9px 12px',
      borderRadius: '10px',
      border: 'none',
      background: selected ? 'var(--dsw-alias-interactive-bg-hover)' : 'transparent',
      color: 'var(--dsw-alias-label-primary)',
      cursor: 'pointer',
      textAlign: 'left',
      font: 'inherit',
    }) as const,
  divider: {
    height: '1px',
    margin: '6px 8px',
    background: 'var(--dsw-alias-border-l2)',
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
    const menuRef = useRef<HTMLElement | null>(null)

    const refresh = useCallback(() => {
      services
        .list()
        .then(setRows)
        .catch(() => setRows([]))
    }, [services])

    // Anchor the menu to the chip on open, flipping above when it would overflow.
    useEffect(() => {
      if (!open) return
      setQuery('')
      setError(undefined)
      refresh()
      const el = anchorRef?.current
      if (!el) return
      const r = el.getBoundingClientRect()
      const estimated = Math.min(window.innerHeight * 0.6, 420)
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

    const run = useCallback(
      (action: () => Promise<void>) => {
        setError(undefined)
        action().catch((reason: unknown) => {
          setError(reason instanceof Error ? reason.message : t.failed)
        })
      },
      [t.failed],
    )

    const chooseFolder = () =>
      run(async () => {
        const path = await services.pickDirectory()
        const workspace = await services.createWorkspace(path)
        onClose()
        onPick(workspace.workspaceId)
      })

    const chooseNoProject = () =>
      run(async () => {
        const sessionId = await services.createUngroupedSession()
        onClose()
        services.openSession(sessionId)
      })

    if (!open || !pos) return null

    return jsx('div', {
      ref: menuRef,
      style: { ...S.menu, left: `${pos.left}px`, top: `${pos.top}px` },
      role: 'menu',
      'data-proteus-code': 'workspace-picker',
      children: [
        jsx('input', {
          key: 'search',
          style: S.search,
          placeholder: t.search,
          value: query,
          'aria-label': t.search,
          onChange: (e: { target: { value: string } }) => setQuery(e.target.value),
        }),
        visible.length === 0
          ? jsx('div', { key: 'empty', style: S.empty, children: t.empty })
          : visible.map((row) =>
              jsx(
                'button',
                {
                  key: row.workspaceId,
                  type: 'button',
                  role: 'menuitem',
                  style: S.row(row.workspaceId === selectedId),
                  onClick: () => {
                    onClose()
                    onPick(row.workspaceId)
                  },
                  children: [
                    jsx('span', { key: 'i', style: { display: 'flex' }, children: FolderIcon() }),
                    jsx('span', { key: 't', children: row.title }),
                    row.workspaceId === selectedId
                      ? jsx('span', { key: 'c', style: S.check, children: '✓' })
                      : null,
                  ],
                },
                row.workspaceId,
              ),
            ),
        jsx('div', { key: 'd', style: S.divider }),
        jsx(
          'button',
          {
            key: 'folder',
            type: 'button',
            role: 'menuitem',
            style: S.row(false),
            onClick: chooseFolder,
            children: [
              jsx('span', { key: 'i', style: { display: 'flex' }, children: FolderIcon() }),
              jsx('span', { key: 't', children: t.openFolder }),
            ],
          },
        ),
        jsx(
          'button',
          {
            key: 'noproject',
            type: 'button',
            role: 'menuitem',
            style: S.row(false),
            onClick: chooseNoProject,
            children: [
              jsx('span', { key: 'i', style: { display: 'flex' }, children: ChatIcon() }),
              jsx('span', { key: 't', children: t.noProject }),
            ],
          },
        ),
        error ? jsx('div', { key: 'err', style: S.error, children: error }) : null,
      ],
    })
  }
}
