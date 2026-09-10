/**
 * Renderer diagnostics, used by the smoke path and by hand during UI work.
 *
 * Two optional, environment-gated capabilities:
 *   - `PROTEUS_CODE_DARK=1` forces the dark scheme before probing, so both
 *     palettes can be checked without clicking through settings.
 *   - `PROTEUS_CODE_PROBE=1` reports the resolved theme tokens and the state of
 *     key controls.
 *
 * This exists because the whole UI is restyled through tokens: verifying that
 * requires reading COMPUTED values, not reading the source. A token typo or a
 * token family the override forgot is invisible in a screenshot and obvious in
 * the probe output.
 *
 * @module @proteus-code/desktop/probe
 */

import type { BrowserWindow } from 'electron'

/** Report whether the dark scheme is active and what the brand slots rendered. */
export interface ProbeResult {
  readonly vars: Record<string, string>
  readonly theme: { darkAttr: boolean }
  readonly computed: Record<string, unknown>
  readonly slots: Record<string, unknown>
  readonly composer: Record<string, unknown>
  readonly locate: readonly unknown[]
}

/** The script evaluated in the renderer; returns a plain JSON object. */
const PROBE_SCRIPT = `(() => {
  const css = (sel) => {
    const el = document.querySelector(sel);
    if (!el) return null;
    const s = getComputedStyle(el);
    return { bg: s.backgroundColor, color: s.color, border: s.borderColor };
  };
  const bodyProto = getComputedStyle(document.body);
  const token = (name) => bodyProto.getPropertyValue(name).trim();
  // Find where a given string lives, so an unstyled/owned-by-upstream string can
  // be located without guessing at class names.
  const locate = (needle) => {
    const out = [];
    const walk = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    let n;
    while ((n = walk.nextNode())) {
      if (n.nodeValue && n.nodeValue.includes(needle)) {
        const el = n.parentElement;
        out.push({
          needle,
          tag: el?.tagName,
          cls: el?.className && String(el.className).slice(0, 80),
        });
      }
    }
    return out;
  };
  const editable = document.querySelector('[contenteditable]');
  const area = document.querySelector('textarea');
  const host = editable || area;
  const send = [...document.querySelectorAll('button')].find((b) =>
    (b.getAttribute('aria-label') || '').match(/发送|send/i),
  );
  return {
    vars: {
      bgBase: token('--dsw-alias-bg-base'),
      bgLayer1: token('--dsw-alias-bg-layer-1'),
      sidebarFill: token('--dsw-specific-sidebar-fill'),
      labelPrimary: token('--dsw-alias-label-primary'),
      labelSecondary: token('--dsw-alias-label-secondary'),
      brandPrimary: token('--dsw-alias-brand-primary'),
      buttonPrimary: token('--dsw-alias-button-primary-fill'),
    },
    theme: { darkAttr: document.body.hasAttribute('data-ds-dark-theme') },
    computed: {
      body: css('body'),
      sidebar: css('aside') || css('[class*="sidebar"]'),
    },
    slots: {
      brandMark: Boolean(document.querySelector('[aria-label="proteus code"]')),
      brandName: document.body.innerText.includes('CODE'),
    },
    composer: {
      found: Boolean(host),
      contentEditable: editable ? editable.getAttribute('contenteditable') : null,
      sendDisabled:
        send === undefined
          ? null
          : send.hasAttribute('disabled') || send.getAttribute('aria-disabled') === 'true',
      placeholder:
        host?.getAttribute('data-placeholder') ||
        host?.getAttribute('placeholder') ||
        host?.closest('[data-placeholder]')?.getAttribute('data-placeholder') ||
        null,
    },
    locate: [...locate('DeepSeek')].slice(0, 6),
  };
})()`

/**
 * Force the dark scheme, for capturing the dark palette without a settings click.
 *
 * The theme plugin re-asserts its own attribute on change, so this only sticks
 * long enough to probe and capture. Prefer persisting `ui-theme.preference` in
 * the Harness home's `settings.yaml` when the result must be a true user state.
 */
export async function forceDarkScheme(window: BrowserWindow): Promise<void> {
  await window.webContents.executeJavaScript(
    `(() => { document.body.setAttribute('data-ds-dark-theme', ''); return true })()`,
  )
  await new Promise((resolve) => setTimeout(resolve, 800))
}

/** Read theme tokens and control state from the renderer. */
export async function readProbe(window: BrowserWindow): Promise<ProbeResult> {
  return (await window.webContents.executeJavaScript(PROBE_SCRIPT)) as ProbeResult
}

/**
 * Report the stable hooks on visually significant elements.
 *
 * The client's class names are hashed per build (`_boot_1fywu_3`), so CSS cannot
 * target them. This dump finds what IS stable — ARIA roles, `data-*` flags, and
 * tag structure — and pairs each with the computed background, radius, and
 * backdrop-filter. That is what styling decisions are made from.
 *
 * Only elements that actually paint a surface are reported, so the output stays
 * readable rather than dumping the whole tree.
 */
export async function dumpSurfaces(window: BrowserWindow): Promise<unknown> {
  return await window.webContents.executeJavaScript(`(() => {
    const out = [];
    const seenClass = new Set();
    const walk = (el, depth) => {
      if (out.length > 140) return;
      const s = getComputedStyle(el);
      const paints =
        (s.backgroundColor && s.backgroundColor !== 'rgba(0, 0, 0, 0)') ||
        (s.backdropFilter && s.backdropFilter !== 'none') ||
        (s.borderTopWidth !== '0px' && s.borderTopStyle !== 'none');
      const r = el.getBoundingClientRect();
      if (paints && r.width > 40 && r.height > 20) {
        // Stable identity: role, then the semantic data-* flags, then exact class
        // list (reported so a hash pattern can be recognized, not relied on).
        const attrs = [...el.attributes]
          .filter((a) => a.name === 'role' || a.name.startsWith('aria-') || a.name.startsWith('data-'))
          .map((a) => a.name + (a.value.length < 30 ? '=' + a.value : ''))
          .slice(0, 5);
        out.push({
          depth,
          tag: el.tagName.toLowerCase(),
          attrs,
          cls: typeof el.className === 'string' ? el.className.slice(0, 60) : '',
          bg: s.backgroundColor,
          radius: s.borderRadius,
          blur: s.backdropFilter,
          border: s.borderTopWidth + ' ' + s.borderTopColor,
          box: Math.round(r.width) + 'x' + Math.round(r.height),
          pos: s.position,
          z: s.zIndex,
        });
        seenClass.add(el.className);
      }
      for (const child of Array.from(el.children)) walk(child, depth + 1);
    };
    const root = document.querySelector('#root') || document.body;
    walk(root, 0);
    // Structural landmarks + which candidate selectors actually resolve, so the
    // brand stylesheet targets real hooks instead of guessed class names.
    const frame = document.querySelector('[data-rightbar-collapsed]');
    const frameKids = frame
      ? [...frame.children].map((el, i) => {
          const r = el.getBoundingClientRect();
          return { i, tag: el.tagName.toLowerCase(), cls: String(el.className).slice(0, 40), box: Math.round(r.width) + 'x' + Math.round(r.height) };
        })
      : [];
    const candidates = {
      frame: '[data-rightbar-collapsed]',
      composerCard: '[data-composer-card]',
      phase: '[data-phase]',
      roleDialog: '[role=dialog]',
      roleMenu: '[role=menu]',
      dataState: '[data-state]',
      menuish: '[data-state][role]',
      frameFirstChild: '[data-rightbar-collapsed] > :first-child',
      frameDropdownWrapper: '[data-radix-popper-content-wrapper]',
      popoverLike: '[data-side]',
      tooltipLike: '[data-tip]',
    };
    const resolved = {};
    for (const [k, sel] of Object.entries(candidates)) {
      try { resolved[k] = document.querySelectorAll(sel).length; } catch { resolved[k] = -1; }
    }
    return { count: out.length, caps: {
      backdropFilter: CSS.supports('backdrop-filter', 'blur(10px) saturate(150%)'),
      webkitBackdropFilter: CSS.supports('-webkit-backdrop-filter', 'blur(10px)'),
      backdropUrl: CSS.supports('backdrop-filter', 'url(#x)'),
      colorMix: CSS.supports('color', 'color-mix(in oklab, red 50%, blue)'),
      rcs: CSS.supports('color', 'rgb(from red r g b / 50%)'),
      oklch: CSS.supports('color', 'oklch(0.7 0.1 250)'),
      has: CSS.supports('selector(:has(a))'),
      insetShadow: CSS.supports('box-shadow', 'inset 0 1px 0 rgba(255,255,255,.8)'),
    }, frameKids, resolved, surfaces: out };
  })()`)
}
