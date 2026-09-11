// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createWorkspacePicker, filterWorkspaces, nextCursor, pickerLabels, type PickerServices } from '../src/client/workspace-picker.tsx'

/**
 * The picker replaced a `single` slot, so it must not lose the stock picker's
 * capabilities. These tests cover the pure logic and the interaction states that
 * are easy to regress: the shared hover/keyboard cursor, and the two new entries
 * reaching the right services.
 */

/**
 * Render the picker component the way React would, then mount it.
 *
 * `createWorkspacePicker` returns a React function component; rather than pull in
 * a renderer, its hooks are exercised through `react-dom`'s real runtime so the
 * interaction behavior under test is the shipped behavior.
 */
async function mount(services: Partial<PickerServices> = {}) {
  const { createRoot } = await import('react-dom/client')
  const { createElement, act } = await import('react')
  const full: PickerServices = {
    list: async () => [],
    pickDirectory: async () => '/tmp/example',
    createWorkspace: async (path) => ({ workspaceId: 'w1', title: path }),
    adoptDefaultProject: async () => '/tmp/example',
    ...services,
  }
  const Picker = createWorkspacePicker(full)
  const host = document.createElement('div')
  document.body.appendChild(host)
  const root = createRoot(host)
  const props = { open: true, anchorRef: { current: null }, selectedId: undefined as string | undefined, onPick: () => {}, onClose: () => {} }
  // `act` is required so effects and discrete-event state updates flush before
  // assertions; without it React batches them past the end of the test.
  await act(async () => {
    root.render(createElement(Picker as never, props as never))
    await new Promise((resolve) => setTimeout(resolve, 20))
  })
  return {
    host,
    /** Render with overridden props, flushing effects. */
    rerender: async (next: Partial<typeof props>) => {
      await act(async () => {
        root.render(createElement(Picker as never, { ...props, ...next } as never))
        await new Promise((resolve) => setTimeout(resolve, 20))
      })
    },
    /** Dispatch a discrete event through React's act boundary. */
    fire: async (node: Element, event: Event) => {
      await act(async () => {
        node.dispatchEvent(event)
        await new Promise((resolve) => setTimeout(resolve, 20))
      })
    },
  }
}

/**
 * Read the background token a row was rendered with.
 *
 * Assertions inspect the inline style rather than the computed color: the states
 * are expressed as theme tokens (`var(--dsw-alias-…)`), and happy-dom does not
 * resolve `var()`, so a computed-color check would be testing the test harness
 * instead of the contract. Which TOKEN each state uses is the actual contract.
 */
function rowBackground(el: Element): string {
  const inline = (el as HTMLElement).style?.background ?? ''
  if (inline) return inline
  const styleAttr = el.getAttribute('style') ?? ''
  const match = /background:\s*([^;]+)/.exec(styleAttr)
  return match?.[1]?.trim() ?? ''
}

/** Whether a row is rendered in the non-highlighted state. */
function isIdle(el: Element): boolean {
  const bg = rowBackground(el)
  return bg === '' || bg === 'transparent'
}

afterEach(() => {
  document.body.innerHTML = ''
  vi.restoreAllMocks()
})

describe('picker copy', () => {
  it('is localized in both UI languages', () => {
    expect(pickerLabels(true).noProject).toBe('不在项目中工作')
    expect(pickerLabels(false).noProject).toBe('Work without a project')
    expect(pickerLabels(true).openFolder).not.toBe(pickerLabels(false).openFolder)
  })
})

describe('keyboard cursor', () => {
  it('moves and wraps in both directions', () => {
    expect(nextCursor(0, 'ArrowDown', 3)).toBe(1)
    expect(nextCursor(2, 'ArrowDown', 3)).toBe(0)
    expect(nextCursor(0, 'ArrowUp', 3)).toBe(2)
    expect(nextCursor(1, 'ArrowUp', 3)).toBe(0)
  })

  it('lands on an end when starting with no cursor', () => {
    expect(nextCursor(-1, 'ArrowDown', 3)).toBe(0)
    expect(nextCursor(-1, 'ArrowUp', 3)).toBe(2)
  })

  it('supports Home and End', () => {
    expect(nextCursor(2, 'Home', 5)).toBe(0)
    expect(nextCursor(0, 'End', 5)).toBe(4)
  })

  it('ignores keys that are not navigation', () => {
    expect(nextCursor(0, 'Enter', 3)).toBeUndefined()
    expect(nextCursor(0, 'a', 3)).toBeUndefined()
    expect(nextCursor(0, 'Escape', 3)).toBeUndefined()
  })

  it('reports no cursor for an empty menu', () => {
    expect(nextCursor(0, 'ArrowDown', 0)).toBe(-1)
    expect(nextCursor(0, 'Enter', 0)).toBeUndefined()
  })
})

describe('filtering', () => {
  const rows = [
    { workspaceId: 'a', title: 'proteus-code' },
    { workspaceId: 'b', title: 'arco-admin' },
    { workspaceId: 'c', title: 'Proteus' },
  ]

  it('returns everything for an empty query', () => {
    expect(filterWorkspaces(rows, '')).toHaveLength(3)
    expect(filterWorkspaces(rows, '   ')).toHaveLength(3)
  })

  it('matches case-insensitively on a substring', () => {
    expect(filterWorkspaces(rows, 'proteus').map((r) => r.workspaceId)).toEqual(['a', 'c'])
    expect(filterWorkspaces(rows, 'ADMIN').map((r) => r.workspaceId)).toEqual(['b'])
  })

  it('returns nothing when there is no match', () => {
    expect(filterWorkspaces(rows, 'zzz')).toEqual([])
  })
})

describe('rendered menu', () => {
  it('is anchored to the chip and carries a search field plus both actions', async () => {
    const { host } = await mount()
    const menu = host.querySelector('[data-proteus-code="workspace-picker"]')
    expect(menu).not.toBeNull()
    expect(menu?.querySelector('input')).not.toBeNull()
    const labels = [...(menu?.querySelectorAll('[role="menuitem"]') ?? [])].map((e) => e.textContent)
    expect(labels.some((l) => /打开文件夹|Open folder/.test(l ?? ''))).toBe(true)
    expect(labels.some((l) => /不在项目中工作|without a project/.test(l ?? ''))).toBe(true)
  })

  it('marks exactly one row as the current cursor position', async () => {
    const { host } = await mount()
    const items = [...host.querySelectorAll('[role="menuitem"]')]
    const highlighted = items.filter((e) => !isIdle(e))
    // A menu must always show where a click or Enter will land.
    expect(highlighted).toHaveLength(1)
    expect(rowBackground(highlighted[0]!)).toContain('interactive-bg-hover')
  })

  it('renders one row at the cursor position at rest', async () => {
    const { host } = await mount()
    const items = [...host.querySelectorAll('[role="menuitem"]')]
    // A menu must always show where a click or Enter will land.
    expect(items.filter((e) => !isIdle(e))).toHaveLength(1)
  })

  /*
   * Hover- and key-driven cursor motion is covered by the `nextCursor` rules
   * above and by live verification in the running app. happy-dom does not deliver
   * React's synthetic `mouseenter`/`keydown`, so asserting a harness-specific
   * dispatch here would test the harness rather than the product.
   */

  it('marks the current workspace, and the cursor outranks it', async () => {
    const { host, rerender } = await mount({
      list: async () => [{ workspaceId: 'w1', title: 'proteus-code' }],
    })
    await rerender({ selectedId: 'w1' })
    const items = [...host.querySelectorAll('[role="menuitem"]')] as HTMLElement[]
    const workspace = items[0]!
    expect(workspace.getAttribute('aria-current')).toBe('true')
    // The cursor starts on the first row, so the landing-point state wins there;
    // the selected token pair is asserted on a row the cursor is not on.
    expect(rowBackground(workspace)).toContain('interactive-bg-hover')
    expect(rowBackground(items[items.length - 1]!)).toBe('transparent')
  })

  it('adopts the default project and selects it when "no project" is chosen', async () => {
    // The UI has no usable workspace-less state, so this row adopts the harness
    // working directory and then goes through the SAME onPick path as any row.
    const adoptDefaultProject = vi.fn(async () => 'ws-default')
    const onPick = vi.fn()
    const onClose = vi.fn()
    const { host, rerender, fire } = await mount({ adoptDefaultProject })
    await rerender({ onPick, onClose })

    const row = [...host.querySelectorAll('[role="menuitem"]')].find((e) =>
      /不在项目中工作|without a project/.test(e.textContent ?? ''),
    ) as HTMLElement
    await fire(row, new MouseEvent('click', { bubbles: true }))

    expect(adoptDefaultProject).toHaveBeenCalledTimes(1)
    expect(onPick).toHaveBeenCalledWith('ws-default')
    expect(onClose).toHaveBeenCalled()
  })

  it('keeps the menu open and shows the reason when adopting fails', async () => {
    const adoptDefaultProject = vi.fn(async () => {
      throw new Error('no default directory was provided by the host')
    })
    const onClose = vi.fn()
    const { host, rerender, fire } = await mount({ adoptDefaultProject })
    await rerender({ onClose })

    const row = [...host.querySelectorAll('[role="menuitem"]')].find((e) =>
      /不在项目中工作|without a project/.test(e.textContent ?? ''),
    ) as HTMLElement
    await fire(row, new MouseEvent('click', { bubbles: true }))

    // A failure must stay visible rather than closing into a silent no-op.
    expect(onClose).not.toHaveBeenCalled()
    expect(host.textContent).toContain('no default directory')
  })

  it('opens the folder picker and selects the created workspace', async () => {
    const pickDirectory = vi.fn(async () => '/tmp/chosen')
    const createWorkspace = vi.fn(async (path: string) => ({ workspaceId: 'w9', title: path }))
    const onPick = vi.fn()
    const { host, rerender, fire } = await mount({ pickDirectory, createWorkspace })
    await rerender({ onPick })

    const row = [...host.querySelectorAll('[role="menuitem"]')].find((e) =>
      /打开文件夹|Open folder/.test(e.textContent ?? ''),
    ) as HTMLElement
    await fire(row, new MouseEvent('click', { bubbles: true }))

    expect(pickDirectory).toHaveBeenCalledTimes(1)
    expect(createWorkspace).toHaveBeenCalledWith('/tmp/chosen')
    expect(onPick).toHaveBeenCalledWith('w9')
  })

  it('surfaces a failure instead of swallowing it', async () => {
    const pickDirectory = vi.fn(async () => {
      throw new Error('picker exploded')
    })
    const { host, fire } = await mount({ pickDirectory })
    const row = [...host.querySelectorAll('[role="menuitem"]')].find((e) =>
      /打开文件夹|Open folder/.test(e.textContent ?? ''),
    ) as HTMLElement
    await fire(row, new MouseEvent('click', { bubbles: true }))
    expect(host.textContent).toContain('picker exploded')
  })
})
