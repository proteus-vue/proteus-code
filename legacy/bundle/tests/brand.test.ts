import { describe, expect, it } from 'vitest'
import { BRAND, OVERRIDABLE_TOKEN_PREFIXES, PROTEUS_TOKENS } from '../src/client/palette.ts'
import { LENS_FILTER_ID, LIQUID_GLASS_CSS, LENS_SVG } from '../src/client/liquid-glass.ts'
import {
  applyBrandToIndex,
  brandFaviconDataUri,
  brandFaviconSvg,
  PRODUCT_NAME,
} from '../src/brand-assets.ts'

describe('brand tokens', () => {
  it('covers the surfaces that carry the product identity', () => {
    // A spot check across each token family, so a truncated table fails loudly.
    for (const token of [
      '--dsw-alias-bg-base',
      '--dsw-alias-bg-layer-1',
      '--dsw-alias-label-primary',
      '--dsw-alias-label-secondary',
      '--dsw-alias-border-l1',
      '--dsw-alias-brand-primary',
      '--dsw-alias-button-primary-fill',
      '--dsw-alias-interactive-bg-hover',
      '--dsw-alias-markdown-code-block',
      '--dsw-alias-state-error-primary',
      '--dsw-alias-state-success-primary',
      '--dsw-alias-scrollbar-bg-l1',
      '--dsw-alias-tooltip-bg',
    ]) {
      expect(PROTEUS_TOKENS[token], token).toBeDefined()
    }
  })

  it('defines both color schemes for every token', () => {
    for (const [name, modes] of Object.entries(PROTEUS_TOKENS)) {
      expect(modes.light, `${name}.light`).toBeTruthy()
      expect(modes.dark, `${name}.dark`).toBeTruthy()
      // Scheme-independent values are allowed only when they are deliberately
      // so (a transparent fill); every other token must state both schemes.
      const intentional = ['transparent', 'inherit']
      if (!intentional.includes(modes.light)) {
        expect(modes.light, `${name} light/dark differ`).not.toBe(modes.dark)
      }
    }
  })

  it('only overrides harness theme tokens', () => {
    for (const name of Object.keys(PROTEUS_TOKENS)) {
      const ok = OVERRIDABLE_TOKEN_PREFIXES.some((prefix) => name.startsWith(prefix))
      expect(ok, name).toBe(true)
    }
  })

  it('restyles the sidebar through its own token family', () => {
    // The sidebar fill is a `--dsw-specific-*` token, not an alias: overriding
    // only the alias family leaves the sidebar its original color.
    expect(PROTEUS_TOKENS['--dsw-specific-sidebar-fill']).toBeDefined()
    for (const token of [
      '--dsw-specific-sidebar-nav-item-hover',
      '--dsw-specific-sidebar-nav-item-active',
      '--dsw-specific-bubble',
      '--dsw-specific-input-major',
    ]) {
      expect(PROTEUS_TOKENS[token], token).toBeDefined()
    }
  })

  it('uses the brand primary for the button fill so actions match the accent', () => {
    expect(PROTEUS_TOKENS['--dsw-alias-button-primary-fill']?.light).toBe(BRAND.primaryLight)
  })
})

describe('brand favicon', () => {
  it('is a self-contained SVG using the brand gradient', () => {
    const svg = brandFaviconSvg()
    expect(svg.startsWith('<svg')).toBe(true)
    expect(svg).toContain(BRAND.cyan)
    expect(svg).toContain(BRAND.primaryLight)
    expect(svg).toContain(BRAND.violet)
    // No external references: a favicon must not need another request.
    expect(svg).not.toContain('href="http')
  })

  it('renders as a data URI', () => {
    const uri = brandFaviconDataUri()
    expect(uri.startsWith('data:image/svg+xml,')).toBe(true)
    expect(decodeURIComponent(uri.split(',')[1] ?? '')).toContain('<svg')
  })
})

describe('index rebranding', () => {
  const upstream = `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <link rel="icon" type="image/svg+xml" href="./favicon.svg" />
    <title>DeepSeek Harness</title>
    <script type="module" src="./assets/index.js"></script>
  </head>
  <body><div id="root"></div></body>
</html>`

  it('retitles the document', () => {
    const out = applyBrandToIndex(upstream)
    expect(out).toContain(`<title>${PRODUCT_NAME}</title>`)
    expect(out).not.toContain('DeepSeek Harness')
  })

  it('replaces the upstream favicon with the inline brand mark', () => {
    const out = applyBrandToIndex(upstream)
    expect(out).toContain('data:image/svg+xml,')
    expect(out).not.toContain('./favicon.svg')
    // Exactly one icon link remains.
    expect(out.match(/rel="icon"/g)).toHaveLength(1)
  })

  it('injects the brand stylesheet before the closing head tag', () => {
    const out = applyBrandToIndex(upstream)
    const styleAt = out.indexOf('data-proteus-code="brand"')
    const headEnd = out.indexOf('</head>')
    expect(styleAt).toBeGreaterThan(-1)
    expect(styleAt).toBeLessThan(headEnd)
  })

  it('keeps the application scripts intact', () => {
    const out = applyBrandToIndex(upstream)
    expect(out).toContain('src="./assets/index.js"')
    expect(out).toContain('<div id="root"></div>')
  })

  it('adds the brand stylesheet and title when the head has no title or icon', () => {
    const bare = '<html><head><meta charset="utf-8"></head><body></body></html>'
    const out = applyBrandToIndex(bare)
    expect(out).toContain('data-proteus-code="brand"')
    expect(out).toContain('rel="icon"')
    // Still inside the head.
    expect(out.indexOf('data-proteus-code="brand"')).toBeLessThan(out.indexOf('</head>'))
  })

  it('is idempotent: a second pass does not duplicate markup', () => {
    const once = applyBrandToIndex(upstream)
    const twice = applyBrandToIndex(once)
    expect(twice.match(/rel="icon"/g)).toHaveLength(1)
    expect(twice.match(/data-proteus-code="brand"/g)).toHaveLength(1)
    expect(twice.match(/data-proteus-code="glass"/g)).toHaveLength(1)
    expect(twice.match(/id="proteus-glass-defs"/g)).toHaveLength(1)
    expect(twice.match(/<title>/g)).toHaveLength(1)
    expect(twice).toContain(`<title>${PRODUCT_NAME}</title>`)
  })
})

describe('liquid glass layer', () => {
  const upstream = '<html><head><title>x</title></head><body></body></html>'

  it('puts the SVG lens definitions before the stylesheet that references them', () => {
    const out = applyBrandToIndex(upstream)
    const svgAt = out.indexOf('id="proteus-glass-defs"')
    const styleAt = out.indexOf('data-proteus-code="glass"')
    expect(svgAt).toBeGreaterThan(-1)
    expect(styleAt).toBeGreaterThan(-1)
    // Filters must resolve on first paint, not after a reflow.
    expect(svgAt).toBeLessThan(styleAt)
  })

  it('defines a displacement filter so the backdrop refracts rather than only blurring', () => {
    expect(LENS_SVG).toContain(`id="${LENS_FILTER_ID}"`)
    expect(LENS_SVG).toContain('feTurbulence')
    expect(LENS_SVG).toContain('feDisplacementMap')
    // The generated noise must be smoothed, or the warp reads as static.
    expect(LENS_SVG).toContain('feGaussianBlur')
  })

  it('keeps the displacement subtle enough not to warp content', () => {
    const scale = /scale="(\d+)"/.exec(LENS_SVG)?.[1]
    expect(Number(scale)).toBeGreaterThan(0)
    expect(Number(scale)).toBeLessThanOrEqual(16)
  })

  it('paints an ambient backdrop, so the glass has light to transmit', () => {
    // Blur over a flat background reads as flat gray, not glass.
    expect(LIQUID_GLASS_CSS).toContain('[data-rightbar-collapsed]')
    expect(LIQUID_GLASS_CSS).toContain('radial-gradient')
    expect(LIQUID_GLASS_CSS).toContain('background-image')
  })

  it('gives every glass surface a specular rim and a float shadow', () => {
    // A top inset highlight plus a 1px inset ring is the Apple-style edge: the
    // eye reads it as a lit bevel. Drawing it with inset shadows rather than a
    // real border is also what keeps it from shifting layout.
    expect(LIQUID_GLASS_CSS).toMatch(/inset 0 1px 0 0/)
    expect(LIQUID_GLASS_CSS).toMatch(/inset 0 0 0 1px/)
    // A wide, soft lift — Apple's surfaces sit close to the plane.
    expect(LIQUID_GLASS_CSS).toMatch(/0 1[0-9]px 2[0-9]px -1[0-9]px/)
  })

  it('clears the opaque token fill, or the backdrop could never show through', () => {
    // The client surfaces carry an opaque background-color token; a translucent
    // gradient over an opaque fill transmits nothing.
    const glassBase = LIQUID_GLASS_CSS.slice(0, LIQUID_GLASS_CSS.indexOf('[data-composer-card]'))
    expect(glassBase).toContain('background-color: transparent !important')
  })

  it('themes both color schemes', () => {
    expect(LIQUID_GLASS_CSS).toContain('body[data-ds-dark-theme]')
  })

  it('targets only stable hooks, never a hashed class literal', () => {
    // Semantic attributes and CSS-module local-name suffixes are stable; a
    // hard-coded hash like `pI_x6G_sidebarCol` would break on every build.
    expect(LIQUID_GLASS_CSS).toContain('[data-rightbar-collapsed]')
    expect(LIQUID_GLASS_CSS).toContain('[data-composer-card]')
    expect(LIQUID_GLASS_CSS).toContain('[class*="_sidebarCol"]')
    expect(LIQUID_GLASS_CSS).not.toMatch(/class\*?=["']?\s*[A-Za-z0-9_-]{4,}_[A-Za-z]+_/)
  })

  it('gates the expensive refraction behind @supports and honors reduced transparency', () => {
    expect(LIQUID_GLASS_CSS).toContain(`@supports (backdrop-filter: url(#${LENS_FILTER_ID}))`)
    expect(LIQUID_GLASS_CSS).toContain('prefers-reduced-motion')
    expect(LIQUID_GLASS_CSS).toContain('prefers-reduced-transparency')
    // The reduced-transparency block must remove the blur, not just the tint.
    const reduced = LIQUID_GLASS_CSS.slice(LIQUID_GLASS_CSS.indexOf('prefers-reduced-transparency'))
    expect(reduced).toContain('backdrop-filter: none')
  })

  it('can be disabled without touching the rest of the rebrand', () => {
    const out = applyBrandToIndex(upstream, { liquidGlass: false })
    expect(out).not.toContain('data-proteus-code="glass"')
    expect(out).not.toContain('id="proteus-glass-defs"')
    // The brand layer is independent.
    expect(out).toContain('data-proteus-code="brand"')
  })

  it('never puts backdrop-filter, isolation, or stacking changes on the sidebar', () => {
    // Regression, twice over. `backdrop-filter` on the sidebar made it the
    // containing block for the settings modal's fixed overlay (modal clamped to
    // 280px); forcing the blur back on with a pseudo plus child z-index/clearing
    // rules then made sidebar content disappear. The sidebar contains the modal,
    // so it gets a translucent fill and NO filter, isolation, or z-index work.
    const sidebar = LIQUID_GLASS_CSS.slice(
      LIQUID_GLASS_CSS.indexOf('── sidebar'),
      LIQUID_GLASS_CSS.indexOf('── composer'),
    )
    expect(sidebar).not.toMatch(/backdrop-filter/)
    expect(sidebar).not.toMatch(/isolation/)
    expect(sidebar).not.toMatch(/z-index/)
    expect(sidebar).not.toMatch(/position:\s*(relative|absolute|fixed)/)
    // It must still tint through to the ambient field.
    expect(sidebar).toMatch(/color-mix\(in oklab, var\(--dsw-specific-sidebar-fill\)/)
  })

  it('never creates a stacking context on a glazed host', () => {
    expect(LIQUID_GLASS_CSS).not.toContain('isolation: isolate')
  })

  it('leaves overlay host rules free of position overrides', () => {
    // Overlay placement belongs to the client (fixed wrapper, absolute popper).
    const hostRule = /\[data-side\],\s*\[role="dialog"\],\s*\[role="menu"\],\s*\[role="listbox"\] \{([^}]*)\}/.exec(
      LIQUID_GLASS_CSS,
    )
    expect(hostRule, 'overlay host rule should exist').not.toBeNull()
    expect(hostRule?.[1]).not.toMatch(/position:/)
  })
})
