/**
 * Copy overrides.
 *
 * A few user-visible strings belong to plugins whose locale namespace cannot be
 * re-registered (`LocaleRuntime.register` throws on a duplicate namespace +
 * locale, and the owning plugin registers first). Those strings are reachable
 * only in the rendered DOM.
 *
 * This module replaces them by exact text match. Two properties make that
 * acceptable rather than hacky:
 *
 *   - It keys on **text content**, not on class names or DOM structure, so a
 *     restyle cannot break it and it only touches the strings it names.
 *   - It is **reversible**: the disposer restores every original text node, and
 *     a MutationObserver keeps late-rendered copies consistent.
 *
 * Everything that CAN be changed through a supported seam (theme tokens, brand
 * slots, the document title) is, and does not appear here.
 *
 * @module @proteus-code/dsh-proteus-code/client/copy-overrides
 */

/**
 * Exact source text to replacement, per language. Keys are the strings the
 * upstream UI renders; values are proteus-code's own copy.
 *
 * The hero headline is the product's slogan, so it is the one substantive
 * override. Both languages are listed because the active UI language is a user
 * preference.
 */
export const COPY_OVERRIDES: Readonly<Record<string, string>> = {
  // Hero headline (zh / en), owned by the conversation plugin's locale namespace.
  探索未至之境: '一套语义，任意渲染',
  'Into the Unknown': 'One semantic core. Any render engine.',
}

/** The minimal DOM surface this module needs, so it is testable without a browser. */
export interface CopyOverrideHost {
  readonly body: {
    readonly textContent?: string | null
  }
}

/**
 * Apply the copy overrides to a document.
 *
 * @param doc - the document to patch.
 * @param overrides - source text to replacement.
 * @returns a disposer that restores every text node it changed and stops observing.
 */
export function installCopyOverrides(
  doc: Document,
  overrides: Readonly<Record<string, string>> = COPY_OVERRIDES,
): () => void {
  const sources = Object.keys(overrides)
  if (sources.length === 0) return () => {}

  /** Every text node we replaced, with its original value, for exact restore. */
  const applied = new Map<Text, string>()
  let disposed = false

  const patch = (text: Text): void => {
    if (disposed) return
    const value = text.nodeValue ?? ''
    for (const source of sources) {
      if (value.includes(source)) {
        applied.set(text, value)
        text.nodeValue = value.split(source).join(overrides[source] as string)
        return
      }
    }
  }

  /**
   * Walk a subtree and patch matching text nodes.
   *
   * A hand-rolled recursion rather than `createTreeWalker`: the latter's
   * `SHOW_TEXT` filtering is not implemented consistently across DOM
   * implementations, and a walker that silently yields nothing would make this
   * feature appear to work while changing no text.
   */
  const scan = (root: Node): void => {
    if (disposed) return
    if (root.nodeType === Node.TEXT_NODE) {
      const text = root as Text
      if (!applied.has(text)) patch(text)
      return
    }
    if (root.nodeType !== Node.ELEMENT_NODE && root.nodeType !== Node.DOCUMENT_FRAGMENT_NODE) return
    for (const child of Array.from(root.childNodes)) scan(child)  }

  scan(doc.body)

  // Later renders (session switches, re-mounts) re-create the nodes, so keep
  // watching. The observer only observes; it never triggers a second pass.
  const observer = new MutationObserver((records) => {
    if (disposed) return
    for (const record of records) {
      // `addedNodes` is a NodeList; iterate a snapshot so the list is stable
      // while handlers patch text.
      for (const node of Array.from(record.addedNodes)) {
        if (node.nodeType === Node.TEXT_NODE) patch(node as Text)
        else if (node.nodeType === Node.ELEMENT_NODE) scan(node)
      }
    }
  })
  observer.observe(doc.body, { childList: true, subtree: true })

  return () => {
    // A queued callback may already be in flight, so the flag — not just
    // disconnect() — decides whether further patches happen.
    disposed = true
    observer.disconnect()
    for (const [node, original] of applied) node.nodeValue = original
    applied.clear()
  }
}
