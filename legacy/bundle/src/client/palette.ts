/**
 * The proteus-code theme.
 *
 * The harness exposes one supported restyling seam: `ctx.theme.overrideTokens`,
 * which stacks a token layer over the active theme without touching the
 * registered base. Every value is a `{ light, dark }` pair, so one table themes
 * both color schemes.
 *
 * The tokens are the harness's own `--dsw-alias-*` design tokens (backgrounds,
 * labels, borders, interactive fills, markdown surfaces, states). Overriding
 * them restyles the entire product surface at once — chat, sidebar, settings,
 * dialogs, code blocks — because those surfaces only ever read tokens, never
 * literal colors.
 *
 * The palette is Proteus's own: indigo as the primary, with cyan and violet
 * reserved for the brand mark. Light surfaces are cool near-whites; dark
 * surfaces are deep blue-slates rather than neutral black, which keeps the
 * indigo accents legible at both ends.
 *
 * @module @proteus-code/dsh-proteus-code/client/palette
 */

/** One token value per color scheme. */
export interface TokenModes {
  readonly light: string
  readonly dark: string
}

/** A token name to its two-scheme value. */
export type TokenOverrideTable = Record<string, TokenModes>

/** Brand constants shared between the token table and the injected CSS. */
export const BRAND = {
  /** Primary accent, used for interactive emphasis. */
  primaryLight: '#4f46e5',
  primaryDark: '#818cf8',
  /** Deeper/lighter pair for hover states. */
  primaryLightHover: '#4338ca',
  primaryDarkHover: '#a5b4fc',
  /** Cyan and violet terminate the brand gradient. */
  cyan: '#22d3ee',
  violet: '#a855f7',
} as const

/** The full token override table. */
export const PROTEUS_TOKENS: TokenOverrideTable = {
  // ── surfaces ────────────────────────────────────────────────────────────────
  '--dsw-alias-bg-base': { light: '#f5f6fb', dark: '#0a0d14' },
  '--dsw-alias-bg-layer-1': { light: '#ffffff', dark: '#101521' },
  '--dsw-alias-bg-layer-2': { light: '#ffffff', dark: '#141a28' },
  '--dsw-alias-bg-layer-3': { light: '#ffffff', dark: '#1a2132' },
  '--dsw-alias-bg-module-platform': { light: '#eef0f8', dark: '#1b2333' },
  '--dsw-alias-bg-multi-select': { light: '#e9ecf6', dark: '#1e2637' },
  '--dsw-alias-bg-overlay': { light: '#f0f2f9', dark: '#1c2434' },
  '--dsw-alias-bg-skeleton': { light: 'rgba(79,70,229,0.07)', dark: 'rgba(129,140,248,0.10)' },
  '--dsw-alias-bg-mask-1': { light: 'rgba(15,23,42,0.24)', dark: 'rgba(0,0,0,0.45)' },
  '--dsw-alias-bg-mask-2': { light: 'rgba(15,23,42,0.12)', dark: 'rgba(0,0,0,0.28)' },
  '--dsw-alias-bg-mask-3': { light: 'rgba(15,23,42,0.48)', dark: 'rgba(0,0,0,0.62)' },
  '--dsw-alias-bg-mask-photo': { light: 'rgba(15,23,42,0.88)', dark: 'rgba(5,8,14,0.90)' },
  '--dsw-alias-bg-mask-drop': { light: 'rgba(255,255,255,0.70)', dark: 'rgba(16,21,33,0.70)' },

  // ── labels ──────────────────────────────────────────────────────────────────
  '--dsw-alias-label-primary': { light: '#0f172a', dark: '#eef2ff' },
  '--dsw-alias-label-primary-bluish': { light: '#111a33', dark: '#e8edff' },
  '--dsw-alias-label-primary-dimmed': { light: 'rgba(15,23,42,0.72)', dark: 'rgba(238,242,255,0.72)' },
  '--dsw-alias-label-primary-foreground': { light: '#ffffff', dark: '#0a0d14' },
  '--dsw-alias-label-primary-inverted': { light: '#ffffff', dark: '#0f172a' },
  '--dsw-alias-label-secondary': { light: '#475569', dark: '#b7c2e0' },
  '--dsw-alias-label-tertiary': { light: '#64748b', dark: '#8b98b8' },
  '--dsw-alias-label-caption': { light: '#64748b', dark: '#8b98b8' },
  '--dsw-alias-label-dimmed': { light: '#94a3b8', dark: '#5c6884' },
  '--dsw-alias-brand-text': { light: '#4338ca', dark: '#c7d2fe' },

  // ── brand ───────────────────────────────────────────────────────────────────
  '--dsw-alias-brand-primary': { light: BRAND.primaryLight, dark: BRAND.primaryDark },
  '--dsw-alias-brand-primary-invert': { light: '#ffffff', dark: '#0a0d14' },
  '--dsw-alias-brand-primary-new-colorprimary-new-color': {
    light: BRAND.primaryLight,
    dark: BRAND.primaryDark,
  },
  '--dsw-alias-link': { light: BRAND.primaryLightHover, dark: BRAND.primaryDarkHover },

  // ── borders ─────────────────────────────────────────────────────────────────
  '--dsw-alias-border-l1': { light: 'rgba(15,23,42,0.06)', dark: 'rgba(255,255,255,0.06)' },
  '--dsw-alias-border-l2': { light: 'rgba(15,23,42,0.10)', dark: 'rgba(255,255,255,0.10)' },
  '--dsw-alias-border-l2-darkmode-thin': { light: 'rgba(15,23,42,0.08)', dark: 'rgba(255,255,255,0.08)' },
  '--dsw-alias-border-l3': { light: 'rgba(15,23,42,0.14)', dark: 'rgba(255,255,255,0.14)' },
  '--dsw-alias-border-l4': { light: 'rgba(15,23,42,0.18)', dark: 'rgba(255,255,255,0.18)' },
  '--dsw-alias-border-inverted': { light: 'rgba(255,255,255,0.10)', dark: 'rgba(15,23,42,0.10)' },
  '--dsw-alias-border-inverted2': { light: 'rgba(255,255,255,0.16)', dark: 'rgba(15,23,42,0.16)' },

  // ── buttons ─────────────────────────────────────────────────────────────────
  '--dsw-alias-button-primary-fill': { light: BRAND.primaryLight, dark: '#6366f1' },
  '--dsw-alias-button-primary-hover': { light: BRAND.primaryLightHover, dark: BRAND.primaryDark },
  '--dsw-alias-button-primary-dimmed': {
    light: 'rgba(79,70,229,0.38)',
    dark: 'rgba(129,140,248,0.34)',
  },
  '--dsw-alias-button-contrast-fill': { light: '#1e1b4b', dark: '#e0e7ff' },
  '--dsw-alias-button-elevated-fill': { light: '#ffffff', dark: '#1a2132' },
  '--dsw-alias-button-floating-fill': { light: '#ffffff', dark: '#1a2132' },
  '--dsw-alias-button-floating-hover': { light: '#f0f2f9', dark: '#222b3f' },
  '--dsw-alias-button-tool-bar-fill': { light: 'rgba(79,70,229,0.08)', dark: 'rgba(129,140,248,0.12)' },
  '--dsw-alias-button-tool-bar-fill-invisible': { light: 'transparent', dark: 'transparent' },
  '--dsw-alias-button-tool-bar-hover': { light: 'rgba(79,70,229,0.14)', dark: 'rgba(129,140,248,0.20)' },
  '--dsw-alias-button-ghost-active-fill': { light: 'rgba(79,70,229,0.10)', dark: 'rgba(129,140,248,0.16)' },
  '--dsw-alias-button-ghost-active-hover': { light: 'rgba(79,70,229,0.16)', dark: 'rgba(129,140,248,0.22)' },
  '--dsw-alias-button-ghost-active-border': { light: 'rgba(79,70,229,0.24)', dark: 'rgba(129,140,248,0.30)' },
  '--dsw-alias-button-info-fill': { light: 'rgba(79,70,229,0.10)', dark: 'rgba(129,140,248,0.16)' },
  '--dsw-alias-button-info-hover': { light: 'rgba(79,70,229,0.16)', dark: 'rgba(129,140,248,0.22)' },

  // ── interactive ─────────────────────────────────────────────────────────────
  '--dsw-alias-interactive-bg-hover': { light: 'rgba(79,70,229,0.07)', dark: 'rgba(129,140,248,0.12)' },
  '--dsw-alias-interactive-bg-hover-solid': { light: '#eef0fb', dark: '#1d2436' },
  '--dsw-alias-interactive-bg-hover-accent': { light: 'rgba(79,70,229,0.12)', dark: 'rgba(129,140,248,0.18)' },
  '--dsw-alias-interactive-bg-active': { light: 'rgba(79,70,229,0.13)', dark: 'rgba(129,140,248,0.18)' },
  '--dsw-alias-interactive-bg-hover-danger': {
    light: 'rgba(239,68,68,0.10)',
    dark: 'rgba(248,113,113,0.16)',
  },

  // ── markdown / code ─────────────────────────────────────────────────────────
  '--dsw-alias-markdown-inline-code': { light: '#eceefb', dark: '#1b2233' },
  '--dsw-alias-markdown-code-block': { light: '#f6f7fb', dark: '#0c1019' },
  '--dsw-alias-markdown-code-block-banner': { light: '#eef0f8', dark: '#141a28' },
  '--dsw-alias-markdown-tag': { light: 'rgba(79,70,229,0.10)', dark: 'rgba(129,140,248,0.16)' },
  '--dsw-alias-markdown-citation': { light: 'rgba(79,70,229,0.10)', dark: 'rgba(129,140,248,0.16)' },
  '--dsw-alias-markdown-placeholder': { light: '#94a3b8', dark: '#5c6884' },
  '--dsw-alias-markdown-code-segment-selected': {
    light: 'rgba(79,70,229,0.14)',
    dark: 'rgba(129,140,248,0.20)',
  },
  '--dsw-alias-markdown-code-segment-unselected': { light: 'transparent', dark: 'transparent' },

  // ── scrollbars ──────────────────────────────────────────────────────────────
  '--dsw-alias-scrollbar-bg-l1': { light: 'rgba(15,23,42,0.14)', dark: 'rgba(255,255,255,0.14)' },
  '--dsw-alias-scrollbar-bg-l2': { light: 'rgba(15,23,42,0.18)', dark: 'rgba(255,255,255,0.18)' },
  '--dsw-alias-scrollbar-hover-l1': { light: 'rgba(79,70,229,0.40)', dark: 'rgba(129,140,248,0.45)' },
  '--dsw-alias-scrollbar-hover-l2': { light: 'rgba(79,70,229,0.48)', dark: 'rgba(129,140,248,0.52)' },

  // ── states ──────────────────────────────────────────────────────────────────
  '--dsw-alias-state-success-primary': { light: '#0d9488', dark: '#2dd4bf' },
  '--dsw-alias-state-success-secondary': { light: 'rgba(13,148,136,0.14)', dark: 'rgba(45,212,191,0.18)' },
  '--dsw-alias-state-success-tertiary': { light: 'rgba(13,148,136,0.08)', dark: 'rgba(45,212,191,0.10)' },
  '--dsw-alias-state-warn-primary': { light: '#d97706', dark: '#fbbf24' },
  '--dsw-alias-state-warn-label': { light: '#b45309', dark: '#fcd34d' },
  '--dsw-alias-state-warn-secondary': { light: 'rgba(217,119,6,0.14)', dark: 'rgba(251,191,36,0.18)' },
  '--dsw-alias-state-warn-tertiary': { light: 'rgba(217,119,6,0.08)', dark: 'rgba(251,191,36,0.10)' },
  '--dsw-alias-state-error-primary': { light: '#dc2626', dark: '#f87171' },
  '--dsw-alias-state-error-secondary': { light: 'rgba(220,38,38,0.14)', dark: 'rgba(248,113,113,0.18)' },
  '--dsw-alias-state-business-primary': { light: BRAND.primaryLight, dark: BRAND.primaryDark },
  '--dsw-alias-state-business-tertiary': { light: 'rgba(79,70,229,0.08)', dark: 'rgba(129,140,248,0.10)' },

  // ── overlays ────────────────────────────────────────────────────────────────
  '--dsw-alias-tooltip-bg': { light: '#1e293b', dark: '#e2e8f0' },
  '--dsw-alias-toast-bg': { light: '#1e293b', dark: '#1a2132' },

  // ── component-specific surfaces ─────────────────────────────────────────────
  // A second token family the theme owns: component surfaces that are deliberately
  // NOT aliases, because a product may want them distinct from the generic layers.
  // The sidebar fill lives here, which is why the sidebar keeps its own color even
  // when the alias tokens are overridden.
  '--dsw-specific-sidebar-fill': { light: '#ffffff', dark: '#0e121c' },
  '--dsw-specific-sidebar-nav-item-hover': {
    light: 'rgba(79,70,229,0.07)',
    dark: 'rgba(129,140,248,0.10)',
  },
  '--dsw-specific-sidebar-nav-item-active': {
    light: 'rgba(79,70,229,0.11)',
    dark: 'rgba(129,140,248,0.16)',
  },
  '--dsw-specific-sidebar-nav-item-active-accent': {
    light: 'rgba(79,70,229,0.18)',
    dark: 'rgba(129,140,248,0.24)',
  },
  '--dsw-specific-bubble': { light: '#f0f2fb', dark: '#151b29' },
  '--dsw-specific-bubble-highlight': { light: '#e4e8f7', dark: '#1b2333' },
  '--dsw-specific-input-major': { light: '#ffffff', dark: '#141a28' },
  '--dsw-specific-login-input': { light: '#f2f4fa', dark: '#101521' },
  '--dsw-specific-selector': { light: '#eef0f8', dark: '#1b2333' },
  '--dsw-specific-menu': { light: '#ffffff', dark: '#141a28' },
  '--dsw-specific-tip': { light: '#eef0f8', dark: '#1b2333' },
}

/**
 * Token families this table is allowed to carry.
 *
 * `--dsw-alias-*` are the semantic aliases (backgrounds, labels, borders, …);
 * `--dsw-specific-*` are the component surfaces the theme keeps distinct — the
 * sidebar fill, message bubbles, the composer input. Both are theme tokens and
 * both are registered through the same override seam; nothing outside these two
 * families is touched.
 */
export const OVERRIDABLE_TOKEN_PREFIXES = ['--dsw-alias-', '--dsw-specific-'] as const

/**
 * Global CSS the token table cannot express: selection color, focus ring, and
 * the boot loader's branding. Injected into `index.html`'s head by the host
 * face, so it exists before the app script runs.
 *
 * The boot loader is anchored on the stable `[data-dsh-boot]` data attribute
 * rather than its hashed class names; the wordmark is its first text-bearing
 * child. Only this transient loader is addressed structurally, and a failure to
 * match it is cosmetic.
 */
export const BRAND_CSS = `
/* proteus-code brand layer */
::selection { background: ${BRAND.primaryLight}33; }
[data-ds-dark-theme] ::selection { background: ${BRAND.primaryDark}40; }

:where(a, button, [role="button"], input, textarea, [tabindex]):focus-visible {
  outline: 2px solid ${BRAND.primaryLight};
  outline-offset: 2px;
  border-radius: 6px;
}
[data-ds-dark-theme] :where(a, button, [role="button"], input, textarea, [tabindex]):focus-visible {
  outline-color: ${BRAND.primaryDark};
}

/* Boot loader: replace the stock wordmark and tint the spinner. */
[data-dsh-boot] > div > div:first-child {
  font-size: 0 !important;
  letter-spacing: 0 !important;
}
[data-dsh-boot] > div > div:first-child::after {
  content: 'proteus code';
  font-size: 13px;
  font-weight: 600;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  background: linear-gradient(90deg, ${BRAND.cyan}, ${BRAND.primaryLight} 55%, ${BRAND.violet});
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
}
[data-dsh-boot-spinner] {
  border-top-color: ${BRAND.primaryLight} !important;
}
`
