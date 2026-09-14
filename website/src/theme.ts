// src/theme.ts —— 主题与外观（**取值全部来自 TUI 的 theme.rs / appearance.rs**）
//
// 为什么要在官网复刻 TUI 的主题系统：站点本身就该像产品。TUI 的 `ctrl+t` 能切 9 套
// 主题、`/background` 能切 4 种纹理、`/logo` 能切 4 种词标 —— 官网把这些**真的做出来**，
// 比"放一张终端截图"更能说明产品是什么。
//
// ★取值纪律：每个色值都从 `crates/neo-host-tui/src/theme.rs` 的 `ThemeName` 抄来，
//   不自行调色。所以官网切换主题时，切到的是**产品里那套真实的主题**。

export type ThemeName =
  | 'neo'
  | 'opencode'
  | 'nord'
  | 'gruvbox'
  | 'rosepine'
  | 'tokyonight'
  | 'catppuccin'
  | 'light'
  | 'terminal'

export interface Theme {
  /** 显示名 */
  label: string
  /** 一句话风格说明（Light 与 Terminal 是「风格包」而非单纯换色） */
  note: string
  primary: string
  accent: string
  success: string
  error: string
  warning: string
  info: string
  text: string
  muted: string
  border: string
  borderActive: string
  bgPanel: string
  bgElement: string
  bgSelected: string
  backdrop: string
  /** 亮色主题：整屏亮底 + 深色文字 */
  light: boolean
  /** 方角：Terminal 的 CRT 风格（边框/圆角全变直角） */
  square: boolean
  /** 默认星场开关（Light/Terminal 关掉） */
  stars: boolean
}

export const THEMES: Record<ThemeName, Theme> = {
  // ── NEO 自有配色：紫为主色，紫→品红渐变 ────────────────────────────
  neo: {
    label: 'Neo',
    note: '默认。紫为主色，紫→品红渐变',
    primary: '#a78bfa',
    accent: '#e879f9',
    success: '#6ee7b7',
    error: '#f87171',
    warning: '#fbbf24',
    info: '#7dd3fc',
    text: '#ededed',
    muted: '#8a8a94',
    border: '#454552',
    borderActive: '#5c5c6e',
    bgPanel: '#201e2a',
    bgElement: '#2a2736',
    bgSelected: '#332f45',
    backdrop: '#0a0a0c',
    light: false,
    square: false,
    stars: true,
  },
  opencode: {
    label: 'OpenCode',
    note: '参考主题：暖橙主色',
    primary: '#fab283',
    accent: '#9d7cd8',
    success: '#7fd88f',
    error: '#e06c75',
    warning: '#f5a742',
    info: '#56b6c2',
    text: '#eeeeee',
    muted: '#808080',
    border: '#484848',
    borderActive: '#606060',
    bgPanel: '#242222',
    bgElement: '#2e2b29',
    bgSelected: '#3a3532',
    backdrop: '#0a0a0c',
    light: false,
    square: false,
    stars: true,
  },
  nord: {
    label: 'Nord',
    note: '冷色极简',
    primary: '#88c0d0',
    accent: '#8fbcbb',
    success: '#a3be8c',
    error: '#bf616a',
    warning: '#d08770',
    info: '#81a1c1',
    text: '#eceff4',
    muted: '#8b95a7',
    border: '#434c5e',
    borderActive: '#4c566a',
    bgPanel: '#242a33',
    bgElement: '#2e3540',
    bgSelected: '#3b4252',
    backdrop: '#0a0a0c',
    light: false,
    square: false,
    stars: true,
  },
  gruvbox: {
    label: 'Gruvbox',
    note: '复古暖色',
    primary: '#83a598',
    accent: '#8ec07c',
    success: '#b8bb26',
    error: '#fb4934',
    warning: '#fe8019',
    info: '#fabd2f',
    text: '#ebdbb2',
    muted: '#928374',
    border: '#665c54',
    borderActive: '#7c6f64',
    bgPanel: '#282421',
    bgElement: '#332d28',
    bgSelected: '#3f3833',
    backdrop: '#0a0a0c',
    light: false,
    square: false,
    stars: true,
  },
  rosepine: {
    label: 'Rosé Pine',
    note: '柔和低饱和',
    primary: '#9ccfd8',
    accent: '#ebbcba',
    success: '#31748f',
    error: '#eb6f92',
    warning: '#f6c177',
    info: '#c4a7e7',
    text: '#e0def4',
    muted: '#6e6a86',
    border: '#403d52',
    borderActive: '#524f67',
    bgPanel: '#1f1d28',
    bgElement: '#2a2636',
    bgSelected: '#312e40',
    backdrop: '#0a0a0c',
    light: false,
    square: false,
    stars: true,
  },
  tokyonight: {
    label: 'Tokyo Night',
    note: '深蓝夜色',
    primary: '#82aaff',
    accent: '#c099ff',
    success: '#c3e88d',
    error: '#ff757f',
    warning: '#ff966c',
    info: '#82aaff',
    text: '#c8d3f5',
    muted: '#828bb8',
    border: '#3b4264',
    borderActive: '#545c7e',
    bgPanel: '#1b1f30',
    bgElement: '#242940',
    bgSelected: '#2f3549',
    backdrop: '#0a0a0c',
    light: false,
    square: false,
    stars: true,
  },
  catppuccin: {
    label: 'Catppuccin',
    note: '柔和马卡龙',
    primary: '#89b4fa',
    accent: '#f5c2e7',
    success: '#a6e3a1',
    error: '#f38ba8',
    warning: '#fab387',
    info: '#89dceb',
    text: '#cdd6f4',
    muted: '#9399b2',
    border: '#45475a',
    borderActive: '#585b70',
    bgPanel: '#1e1e27',
    bgElement: '#27273a',
    bgSelected: '#313244',
    backdrop: '#060911',
    light: false,
    square: false,
    stars: true,
  },
  // ── 风格包（不只是换色）────────────────────────────────────────────
  light: {
    label: 'Light',
    note: '亮色：整屏亮底 + 深色文字',
    primary: '#4c60d4',
    accent: '#9d5cd8',
    success: '#1a7f4b',
    error: '#c0392b',
    warning: '#b8770a',
    info: '#1f6f9c',
    text: '#2a2c32',
    muted: '#6e727a',
    border: '#c9c9c5',
    borderActive: '#a8a8a2',
    bgPanel: '#f0f0ee',
    bgElement: '#e4e4e1',
    bgSelected: '#d6d6d2',
    backdrop: '#e6e6e4',
    light: true,
    square: false,
    stars: false,
  },
  terminal: {
    label: 'Terminal',
    note: 'CRT 磷绿：方角边框、无纹理、通体磷绿',
    primary: '#53ff9c',
    accent: '#a8ffc8',
    success: '#53ff9c',
    error: '#ff5f56',
    warning: '#ffd75f',
    info: '#5fd7ff',
    text: '#2ee574',
    muted: '#1d9a52',
    border: '#1d9a52',
    borderActive: '#2ee574',
    bgPanel: '#071a0e',
    bgElement: '#0b2413',
    bgSelected: '#10351c',
    backdrop: '#010503',
    light: false,
    square: true,
    stars: false,
  },
}

/** 与 TUI `ThemeName` 的声明顺序一致（切换时按此顺序循环，对齐 `ctrl+t`） */
export const THEME_ORDER: ThemeName[] = [
  'neo',
  'opencode',
  'nord',
  'gruvbox',
  'rosepine',
  'tokyonight',
  'catppuccin',
  'light',
  'terminal',
]

// ── 背景纹理（对齐 TUI `/background`）────────────────────────────────
export type BgKind = 'stars' | 'dots' | 'diagonal' | 'none'

export const BG_LABEL: Record<BgKind, string> = {
  stars: '星场',
  dots: '点阵',
  diagonal: '斜纹',
  none: '纯色',
}

export const BG_ORDER: BgKind[] = ['stars', 'dots', 'diagonal', 'none']

// ── 词标（对齐 TUI `/logo`，字形取自 lib.rs 的 LOGO_LARGE / appearance.rs 的 LOGO_SMALL）
export const LOGO_LARGE = [
  '███╗   ██╗ ███████╗ ██████╗ ',
  '████╗  ██║ ██╔════╝██╔═══██╗',
  '██╔██╗ ██║ █████╗  ██║   ██║',
  '██║╚██╗██║ ██╔══╝  ██║   ██║',
  '██║ ╚████║ ███████╗╚██████╔╝',
  '╚═╝  ╚═══╝ ╚══════╝ ╚═════╝ ',
]

export const LOGO_SMALL = ['█▀▀█ █▀▀▀ █▀▀█', '█  █ █▀▀▀ █  █', '▀▀▀▀ ▀▀▀▀ ▀▀▀▀']

export const LOGO_MINIMAL = ['◈ NEO']
