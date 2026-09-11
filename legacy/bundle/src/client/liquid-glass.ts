/**
 * The proteus-code liquid-glass surface layer.
 *
 * Injected as one stylesheet into the served `index.html`, so it exists before
 * the application renders.
 *
 * ## The Apple-style reading of "liquid glass"
 *
 * Depth and edge light, not colour. A rainbow backdrop reads as a gradient
 * wallpaper; Apple's glass reads as a *material* because four things are true,
 * and this sheet does exactly these:
 *
 *   1. **A near-neutral backdrop.** A pale, faintly brand-tinted field. Its job
 *      is to give the glass something with luminance to transmit, not to be
 *      looked at. It is deliberately desaturated: strong colour behind glass
 *      turns the glass into a colour filter.
 *   2. **High blur with saturation lift.** `blur(24px) saturate(180%)` — the blur
 *      is generous (glass has thickness) and the saturation compensates for what
 *      the blur washes out.
 *   3. **Specular rim.** A bright inset highlight on the light-facing edges, a
 *      darker one opposite, and a 1px outer edge. This is the entire illusion.
 *   4. **Restrained float.** A soft, wide, low-opacity shadow. Apple's surfaces
 *      sit close to the plane; a hard drop shadow breaks the effect.
 *
 * ## Two hard-won implementation constraints
 *
 * **`backdrop-filter` creates a containing block for `position: fixed`
 * descendants.** This is the bug that broke the settings modal: the sidebar
 * carries the filter, the modal is a `fixed` descendant of it, so the modal was
 * clamped to the 280px sidebar instead of the viewport. Fix: the filter lives on
 * a `::before` pseudo-element, which has no descendants and therefore cannot be
 * any element's containing block. `isolation: isolate` on the host keeps the
 * pseudo's negative z-index inside the host's own stacking context.
 *
 * **An opaque fill hides the backdrop.** The client surfaces carry an opaque
 * token fill (white). A translucent gradient over an opaque fill transmits
 * nothing, so the fill is cleared first.
 *
 * ## Targeting discipline
 *
 * Client class names are hashed per build (`pI_x6G_sidebarCol`), so only stable
 * hooks are used: semantic data attributes the client sets deliberately, and
 * CSS-module local-name suffixes. Every rule degrades to the element's existing
 * token colour if its hook disappears.
 *
 * @module @proteus-code/dsh-proteus-code/client/liquid-glass
 */

/** Filter id referenced by `backdrop-filter: url(#…)`. */
export const LENS_FILTER_ID = 'proteus-glass-lens'

/** Id of the hidden `<svg>` carrying the filter definitions. */
const LENS_SVG_ID = 'proteus-glass-defs'

/**
 * The hidden SVG holding the lens filter.
 *
 * `feTurbulence` builds a smooth organic field, a light blur smooths it into a
 * low-frequency wobble, and `feDisplacementMap` offsets the backdrop by it. The
 * scale is small (8): enough to bend straight lines passing behind an edge — the
 * cue the eye reads as refraction — without warping content.
 */
export const LENS_SVG = `<svg id="${LENS_SVG_ID}" width="0" height="0" aria-hidden="true" focusable="false" style="position:absolute;width:0;height:0;overflow:hidden">
  <defs>
    <filter id="${LENS_FILTER_ID}" x="-15%" y="-15%" width="130%" height="130%" color-interpolation-filters="sRGB">
      <feTurbulence type="fractalNoise" baseFrequency="0.005 0.009" numOctaves="2" seed="11" result="noise"/>
      <feGaussianBlur in="noise" stdDeviation="5" result="soft"/>
      <feDisplacementMap in="SourceGraphic" in2="soft" scale="8" xChannelSelector="R" yChannelSelector="G"/>
    </filter>
  </defs>
</svg>`

/**
 * Backdrop.
 *
 * A pale field with three very soft brand-tinted pools. Opacities are low
 * (20-34%) on purpose: seen through 24px of blur these become a gentle tint,
 * which is the effect. Any stronger and the glass turns into a colour wash.
 * Pools sit in the corners so the centre stays neutral behind copy.
 */
const AMBIENT = `
/* ── backdrop ─────────────────────────────────────────────────────────────── */
[data-rightbar-collapsed] {
  background-color: transparent !important;
  background-image:
    radial-gradient(1100px 760px at -4% -14%, color-mix(in oklab, #6366f1 30%, transparent), transparent 64%),
    radial-gradient(900px 620px at 104% -4%, color-mix(in oklab, #0ea5e9 22%, transparent), transparent 62%),
    radial-gradient(1000px 800px at 94% 110%, color-mix(in oklab, #8b5cf6 26%, transparent), transparent 64%),
    linear-gradient(168deg, #f7f8fc 0%, #f2f3f9 52%, #eff0f7 100%) !important;
}
body[data-ds-dark-theme] [data-rightbar-collapsed] {
  background-image:
    radial-gradient(1100px 760px at -6% -16%, color-mix(in oklab, #4f46e5 44%, transparent), transparent 62%),
    radial-gradient(900px 620px at 106% -6%, color-mix(in oklab, #0891b2 30%, transparent), transparent 60%),
    radial-gradient(1000px 800px at 96% 112%, color-mix(in oklab, #7c3aed 40%, transparent), transparent 62%),
    linear-gradient(168deg, #0b0e15 0%, #090b11 52%, #0b0d16 100%) !important;
}

/* The content column is transparent so the backdrop reads as one continuous
   field. Chat bubbles and the composer carry their own contrast. */
[data-rightbar-collapsed] > [class*="_centerCol"],
[data-rightbar-collapsed] > [class*="_centerCol"] > *,
[data-rightbar-collapsed] [class*="_root"][data-phase],
[data-rightbar-collapsed] [class*="_root"][data-phase] > * {
  background-color: transparent !important;
}
[data-rightbar-collapsed] [class*="_root"][data-phase] {
  background-image: none !important;
}
`

/**
 * Shared material constants, inlined into each surface rule.
 *
 * The edge is drawn with an inset ring rather than a real `border`, so it cannot
 * shift layout or box-sizing.
 */
const LIGHT_MATERIAL = `
backdrop-filter: blur(24px) saturate(180%);
-webkit-backdrop-filter: blur(24px) saturate(180%);
background-image: linear-gradient(
  152deg,
  color-mix(in oklab, white 52%, transparent) 0%,
  color-mix(in oklab, white 22%, transparent) 46%,
  color-mix(in oklab, white 34%, transparent) 100%
);
box-shadow:
  inset 0 1px 0 0 color-mix(in oklab, white 95%, transparent),
  inset 0 0 0 1px color-mix(in oklab, white 50%, transparent),
  0 1px 2px color-mix(in oklab, #0b1020 6%, transparent),
  0 10px 28px -10px color-mix(in oklab, #1e1b4b 16%, transparent);
`

const DARK_MATERIAL = `
backdrop-filter: blur(26px) saturate(160%);
-webkit-backdrop-filter: blur(26px) saturate(160%);
background-image: linear-gradient(
  152deg,
  color-mix(in oklab, white 13%, transparent) 0%,
  color-mix(in oklab, white 4%, transparent) 48%,
  color-mix(in oklab, white 9%, transparent) 100%
);
box-shadow:
  inset 0 1px 0 0 color-mix(in oklab, white 30%, transparent),
  inset 0 0 0 1px color-mix(in oklab, white 13%, transparent),
  0 2px 6px color-mix(in oklab, #000 40%, transparent),
  0 16px 40px -14px color-mix(in oklab, #000 60%, transparent);
`

/**
 * The glass surfaces.
 *
 * Two different treatments, chosen by one question: **can this element have a
 * `position: fixed` descendant?**
 *
 * A `backdrop-filter` makes its element the containing block for fixed
 * descendants, and a stacking context can strand those descendants behind
 * sibling columns. The settings modal lives inside the sidebar subtree, so the
 * sidebar must not carry a filter, must not be isolated, and must not have its
 * children's stacking rearranged. Attempts to force blur onto it produced two
 * regressions (a clamped modal, then vanishing sidebar content). It therefore
 * uses the treatment that cannot affect layout at all:
 *
 *   - **Sidebar: a translucent fill.** The ambient field shows through the
 *     sidebar's own tint. No filter, no pseudo, no position or z-index change —
 *     so it cannot become a containing block, cannot create a stacking context,
 *     and cannot hide its own content. This is also how Apple's real sidebars
 *     behave: a mostly-opaque material, not a clear pane.
 *   - **Composer and overlays: true glass.** These never contain fixed
 *     descendants, so the material rides a `::before` (which has no descendants
 *     at all and is therefore always safe) and gets the blur plus refraction.
 */
const SURFACES = `
/* ── sidebar: translucent sheet, no filter ─────────────────────────────────── */
[data-rightbar-collapsed] > [class*="_sidebarCol"] {
  background-color: color-mix(in oklab, var(--dsw-specific-sidebar-fill) 82%, transparent) !important;
  background-image: none !important;
  box-shadow: inset -1px 0 0 0 color-mix(in oklab, white 40%, transparent);
}
/* The nav root inside the column paints its own opaque fill on top of the sheet;
   clearing just that one container lets the translucent sheet show. Scoped to
   this single container so no content styling is disturbed. */
[data-rightbar-collapsed] > [class*="_sidebarCol"] > [class*="_root"] {
  background-color: transparent !important;
  background-image: none !important;
}
body[data-ds-dark-theme] [data-rightbar-collapsed] > [class*="_sidebarCol"] {
  box-shadow: inset -1px 0 0 0 color-mix(in oklab, white 9%, transparent);
}

/* ── composer: the primary floating surface ───────────────────────────────── */
[data-composer-card] {
  background-color: transparent !important;
  background-image: none !important;
  border-radius: 22px !important;
  transition: box-shadow 280ms cubic-bezier(0.22, 1, 0.36, 1), transform 280ms cubic-bezier(0.22, 1, 0.36, 1);
}
[data-composer-card]::before {
  content: "";
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  border-radius: inherit;
  ${LIGHT_MATERIAL}
}
[data-composer-card] > * {
  position: relative;
  z-index: 1;
}
body[data-ds-dark-theme] [data-composer-card]::before {
  ${DARK_MATERIAL}
}
[data-composer-card]:focus-within {
  transform: translateY(-1px);
  box-shadow:
    0 2px 6px color-mix(in oklab, #0b1020 8%, transparent),
    0 16px 40px -12px color-mix(in oklab, #4f46e5 30%, transparent);
}
body[data-ds-dark-theme] [data-composer-card]:focus-within {
  box-shadow:
    0 2px 8px color-mix(in oklab, #000 46%, transparent),
    0 20px 48px -14px color-mix(in oklab, #6366f1 40%, transparent);
}

/* ── floating overlays: dropdowns, menus ──────────────────────────────────── */
/* Position untouched: the client owns popper placement. The material rides a
   pseudo so no containing block is created for nested fixed content. */
[data-side],
[role="dialog"],
[role="menu"],
[role="listbox"] {
  background-color: transparent !important;
  background-image: none !important;
}
[data-side]::before,
[role="dialog"]::before,
[role="menu"]::before,
[role="listbox"]::before {
  content: "";
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  border-radius: inherit;
  ${LIGHT_MATERIAL}
}
[data-side] > *,
[role="dialog"] > *,
[role="menu"] > *,
[role="listbox"] > * {
  position: relative;
  z-index: 1;
}
body[data-ds-dark-theme] [data-side]::before,
body[data-ds-dark-theme] [role="dialog"]::before,
body[data-ds-dark-theme] [role="menu"]::before,
body[data-ds-dark-theme] [role="listbox"]::before {
  ${DARK_MATERIAL}
}

/* ── sidebar controls ─────────────────────────────────────────────────────── */
[data-rightbar-collapsed] > [class*="_sidebarCol"] button {
  border-radius: 12px;
  transition: background-color 200ms ease, box-shadow 200ms ease;
}
[data-rightbar-collapsed] > [class*="_sidebarCol"] button:hover:not(:disabled) {
  background-color: color-mix(in oklab, black 5%, transparent) !important;
}
body[data-ds-dark-theme] [data-rightbar-collapsed] > [class*="_sidebarCol"] button:hover:not(:disabled) {
  background-color: color-mix(in oklab, white 8%, transparent) !important;
}
`

/**
 * Refraction, via the SVG lens filter.
 *
 * Gated on `@supports`. Applied only to the small floating surfaces: an
 * `url()` backdrop filter re-evaluates on every paint, so it is kept off the
 * full-height sidebar. This is the one genuinely expensive effect in the sheet.
 */
const LENS = `
@supports (backdrop-filter: url(#${LENS_FILTER_ID})) {
  [data-composer-card]::before,
  [data-side]::before,
  [role="dialog"]::before {
    backdrop-filter: blur(24px) saturate(180%) url(#${LENS_FILTER_ID});
    -webkit-backdrop-filter: blur(24px) saturate(180%) url(#${LENS_FILTER_ID});
  }
  body[data-ds-dark-theme] [data-composer-card]::before,
  body[data-ds-dark-theme] [data-side]::before,
  body[data-ds-dark-theme] [role="dialog"]::before {
    backdrop-filter: blur(26px) saturate(160%) url(#${LENS_FILTER_ID});
    -webkit-backdrop-filter: blur(26px) saturate(160%) url(#${LENS_FILTER_ID});
  }
}

/* The hero mark carries a soft brand glow. */
[data-phase="hero"] [aria-label="proteus code"] {
  filter: drop-shadow(0 5px 16px color-mix(in oklab, #6366f1 40%, transparent));
}
`

/**
 * Accessibility fallbacks, last so they win.
 *
 * `prefers-reduced-transparency` removes the blur itself and restores an opaque
 * fill — dropping only the tint would leave unreadable text over a busy backdrop.
 */
const ACCESSIBILITY = `
@media (prefers-reduced-motion: reduce) {
  [data-composer-card] { transition: none; }
  [data-composer-card]:focus-within { transform: none; }
}

@media (prefers-reduced-transparency: reduce) {
  [data-composer-card]::before,
  [data-side]::before,
  [role="dialog"]::before,
  [role="menu"]::before,
  [role="listbox"]::before {
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
  }
  [data-rightbar-collapsed] > [class*="_sidebarCol"] {
    background-color: var(--dsw-specific-sidebar-fill) !important;
  }
  [data-composer-card]::before,
  [data-side]::before,
  [role="dialog"]::before,
  [role="menu"]::before,
  [role="listbox"]::before {
    background-image: none;
    background-color: var(--dsw-alias-bg-layer-2);
  }
}
`

/**
 * The complete liquid-glass stylesheet.
 *
 * Order: backdrop (glass needs light to carry), surfaces, refraction, then the
 * accessibility overrides last so they take precedence.
 */
export const LIQUID_GLASS_CSS = ['/* proteus-code liquid glass */', AMBIENT, SURFACES, LENS, ACCESSIBILITY].join('\n')

/** Whether the SVG lens definitions are already present in a document. */
export function hasLensDefs(doc: Document, id: string = LENS_SVG_ID): boolean {
  return doc.getElementById(id) !== null
}
