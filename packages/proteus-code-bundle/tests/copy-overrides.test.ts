// @vitest-environment happy-dom
import { describe, expect, it } from 'vitest'
import { COPY_OVERRIDES, installCopyOverrides } from '../src/client/copy-overrides.ts'

/**
 * The copy overrides key on exact text content, so these tests check the three
 * properties that make that safe: only the named strings change, the change is
 * reversible, and late-rendered copies are covered.
 */
describe('copy overrides', () => {
  it('replaces the hero headline in both languages', () => {
    document.body.innerHTML = '<div><h1>探索未至之境</h1><span>Into the Unknown</span></div>'
    const dispose = installCopyOverrides(document)
    expect(document.body.textContent).toContain('一套语义，任意渲染')
    expect(document.body.textContent).toContain('One semantic core. Any render engine.')
    dispose()
  })

  it('leaves unrelated copy untouched', () => {
    document.body.innerHTML = '<div><h1>探索未至之境</h1><p>新会话</p><p>设置</p></div>'
    const dispose = installCopyOverrides(document)
    expect(document.body.textContent).toContain('新会话')
    expect(document.body.textContent).toContain('设置')
    dispose()
  })

  it('restores the original text on dispose', () => {
    document.body.innerHTML = '<div id="a">探索未至之境</div>'
    const dispose = installCopyOverrides(document)
    expect(document.querySelector('#a')?.textContent).toBe('一套语义，任意渲染')
    dispose()
    expect(document.querySelector('#a')?.textContent).toBe('探索未至之境')
  })

  it('handles the string appearing inside a larger text node', () => {
    document.body.innerHTML = '<div>Welcome — 探索未至之境 — enjoy</div>'
    const dispose = installCopyOverrides(document)
    expect(document.body.textContent).toBe('Welcome — 一套语义，任意渲染 — enjoy')
    dispose()
    expect(document.body.textContent).toBe('Welcome — 探索未至之境 — enjoy')
  })

  it('covers a late-rendered copy added after install', async () => {
    document.body.innerHTML = '<div id="host"></div>'
    const dispose = installCopyOverrides(document)
    const node = document.createElement('h1')
    node.textContent = '探索未至之境'
    document.querySelector('#host')?.appendChild(node)

    // The observer delivers as a microtask.
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(document.querySelector('h1')?.textContent).toBe('一套语义，任意渲染')
    dispose()
    expect(document.querySelector('h1')?.textContent).toBe('探索未至之境')
  })

  it('stops patching after dispose', async () => {
    document.body.innerHTML = '<div id="host"></div>'
    const dispose = installCopyOverrides(document)
    dispose()
    const node = document.createElement('h1')
    node.textContent = '探索未至之境'
    document.querySelector('#host')?.appendChild(node)
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(document.querySelector('h1')?.textContent).toBe('探索未至之境')
  })

  it('leaves the page usable when no override text is present', () => {
    document.body.innerHTML = '<div>新会话</div>'
    expect(() => installCopyOverrides(document)()).not.toThrow()
    expect(document.body.textContent).toBe('新会话')
  })

  it('names only the hero headline as an override', () => {
    // Keep the override list deliberate: every entry is a string the app owns
    // and this bundle cannot reach through a supported seam.
    expect(Object.keys(COPY_OVERRIDES).sort()).toEqual(['Into the Unknown', '探索未至之境'])
  })
})
