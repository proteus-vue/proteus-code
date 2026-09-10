/**
 * Host-side brand assets.
 *
 * Two things the theme token layer cannot reach live in the served
 * `index.html` itself: the document title and the favicon. The harness's
 * structured index-injection rows have no `title` or `link` kind, so this uses
 * `webServer.tapIndex` — the documented escape hatch for "anything not
 * expressible as a row" — and rewrites the three head facts in one pass.
 *
 * The brand stylesheet is inserted immediately before `</head>` so it follows
 * the application's own stylesheets and wins where specificity ties.
 *
 * Registration is gated on `webServer`, so a headless profile mounts the plugin
 * without this module doing anything.
 *
 * @module @proteus-code/dsh-proteus-code/brand-assets
 */

import { BRAND, BRAND_CSS } from './client/palette.ts'
import { LENS_SVG, LIQUID_GLASS_CSS } from './client/liquid-glass.ts'

/** Product name used for the document title and window identity. */
export const PRODUCT_NAME = 'proteus code'

/** Options for {@link applyBrandToIndex}. */
export interface BrandIndexOptions {
  /** Inject the liquid-glass surface layer. Defaults to true. */
  readonly liquidGlass?: boolean
  /** Let the client face take over the hero workspace picker. Defaults to true. */
  readonly workspacePicker?: boolean
}

/**
 * The proteus-code mark as a standalone SVG: a rounded badge with the gradient
 * from the in-app brand slots and the same "P" silhouette.
 */
export function brandFaviconSvg(): string {
  return [
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">',
    '<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">',
    `<stop offset="0%" stop-color="${BRAND.cyan}"/>`,
    `<stop offset="55%" stop-color="${BRAND.primaryLight}"/>`,
    `<stop offset="100%" stop-color="${BRAND.violet}"/>`,
    '</linearGradient></defs>',
    '<rect x="1" y="1" width="30" height="30" rx="9" fill="url(#g)"/>',
    '<path d="M11 23V9h6.2c3.1 0 5.3 1.9 5.3 4.7 0 2.9-2.2 4.8-5.3 4.8H14.6V23H11zm3.6-7.6h2.3c1.3 0 2.1-.7 2.1-1.7s-.8-1.7-2.1-1.7h-2.3v3.4z" fill="#fff"/>',
    '</svg>',
  ].join('')
}

/** The favicon as a data URI, so no extra route or file is needed. */
export function brandFaviconDataUri(): string {
  return `data:image/svg+xml,${encodeURIComponent(brandFaviconSvg())}`
}

/**
 * Rewrite the served index: retitle the document, swap the favicon, and append
 * the brand stylesheet. Each rewrite is independent, so an unexpected upstream
 * markup change degrades one feature instead of breaking the page.
 */
/**
 * Rewrite the served index: retitle the document, swap the favicon, and append
 * the brand stylesheet plus the liquid-glass layer and its SVG lens filter.
 *
 * Each rewrite is independent, so an unexpected upstream markup change degrades
 * one feature instead of breaking the page. Every insertion is guarded by a
 * marker check, so repeated taps stay idempotent.
 */
export function applyBrandToIndex(html: string, options: BrandIndexOptions = {}): string {
  let out = html
  const withGlass = options.liquidGlass !== false

  if (/<title>[^<]*<\/title>/i.test(out)) {
    out = out.replace(/<title>[^<]*<\/title>/i, `<title>${escapeHtml(PRODUCT_NAME)}</title>`)
  }

  const favicon = `<link rel="icon" type="image/svg+xml" href="${brandFaviconDataUri()}" />`
  // Replace the existing icon link when present; otherwise add one.
  if (/<link[^>]*rel=["']icon["'][^>]*>/i.test(out)) {
    out = out.replace(/<link[^>]*rel=["']icon["'][^>]*>/i, favicon)
  } else if (/<\/head>/i.test(out)) {
    out = out.replace(/<\/head>/i, `    ${favicon}\n  </head>`)
  }

  const head: string[] = []
  if (!out.includes('data-proteus-code="brand"')) {
    head.push(`<style data-proteus-code="brand">${BRAND_CSS}</style>`)
  }
  // Forward the runtime choices the browser face needs. The client half gets no
  // plugin config from the harness, so the host half hands it over here.
  if (!out.includes('__PROTEUS_CODE_OPTS__')) {
    const opts = { workspacePicker: options.workspacePicker !== false }
    head.push(`<script>globalThis.__PROTEUS_CODE_OPTS__=${JSON.stringify(opts)}</script>`)
  }
  if (withGlass) {
    // The SVG must precede the stylesheet that references it by id, so the
    // filter resolves on the first paint rather than after a reflow.
    if (!out.includes('id="proteus-glass-defs"')) head.push(LENS_SVG)
    if (!out.includes('data-proteus-code="glass"')) {
      head.push(`<style data-proteus-code="glass">${LIQUID_GLASS_CSS}</style>`)
    }
  }
  if (head.length > 0) {
    const block = head.join('\n    ')
    out = /<\/head>/i.test(out)
      ? out.replace(/<\/head>/i, `    ${block}\n  </head>`)
      : `${block}${out}`
  }

  return out
}

/** Escape a string for use in an HTML text node. */
function escapeHtml(value: string): string {
  return value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;')
}

/** The web server surface this module needs. */
export interface WebServerSeam {
  tapIndex(transform: (html: string) => string): () => void
}

/** Context surface for a nested service-gated registration. */
export interface InjectContext {
  inject(deps: string[], callback: (scope: { webServer: WebServerSeam }) => void): unknown
}

/**
 * Install the brand assets for the browser surface.
 *
 * Gated on `webServer`, so profiles without a web server (headless, sdk) skip it
 * silently rather than failing to load.
 */
export function installBrandAssets(ctx: InjectContext, options: BrandIndexOptions = {}): void {
  ctx.inject(['webServer'], (scope) => {
    scope.webServer.tapIndex((html) => applyBrandToIndex(html, options))
  })
}
