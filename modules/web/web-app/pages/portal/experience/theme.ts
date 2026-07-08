/**
 * Portal design tokens.
 *
 * Ported from the "Customer Portal" design (Layout B — centered, slim tabs,
 * light-default with a dark variant, blush accent, dark "spotlight" cards).
 *
 * The portal renders inside a single scoped root so its tokens never leak into
 * (or inherit from) the host product's global Tailwind theme. Everything is
 * driven by CSS variables so a single token swap re-skins the whole surface,
 * including embeds and overlays.
 *
 * Resolution precedence (highest wins):
 *   1. URL overrides         (?theme=dark&accent=%23C6F94E&radius=Rounded)
 *   2. Tenant branding        (invoicing entity brand color, future portal settings)
 *   3. Built-in defaults      (light · Modern radii · blush)
 */

export type PortalThemeMode = 'light' | 'dark'
export type PortalRoundness = 'Sharp' | 'Modern' | 'Rounded'

/**
 * Curated host-overridable colors. A small, stable subset of the internal
 * `--mtp-*` palette — enough to match a host product's surface/text/borders
 * without exposing (and freezing) every internal token. Each is a hex string.
 */
export interface PortalColorOverrides {
  bg?: string
  surface?: string
  text?: string
  border?: string
}

/** Maps each curated override onto the internal token it sets. */
const TOKEN_BY_COLOR: Record<keyof PortalColorOverrides, string> = {
  bg: '--mtp-bg',
  surface: '--mtp-surface',
  text: '--mtp-text',
  border: '--mtp-border',
}

export interface PortalThemeConfig {
  theme: PortalThemeMode
  roundness: PortalRoundness
  accent: string
  /** Curated per-token overrides (from the embed URL). */
  colors?: PortalColorOverrides
}

export const DEFAULT_ACCENT = '#EFC9C9'

export const DEFAULT_THEME: PortalThemeConfig = {
  theme: 'dark',
  roundness: 'Modern',
  accent: DEFAULT_ACCENT,
}

/** Tokens (raw values) that the host can override for branding. */
export interface PortalBranding {
  /** Display name shown next to the logo in the header. */
  companyName?: string
  /** Absolute URL of the seller logo. */
  logoUrl?: string
  /** Hex accent color, e.g. the invoicing-entity brand color. */
  accent?: string
  /** Default theme mode for this tenant. */
  theme?: PortalThemeMode
  /** Default control roundness for this tenant. */
  roundness?: PortalRoundness
  /** Whether to show the "Powered by Meteroid" footer. */
  showPoweredBy?: boolean
}

const HEX = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i

const isHex = (v: string | null | undefined): v is string => !!v && HEX.test(v)

const h2r = (h: string): [number, number, number] => {
  let s = h.replace('#', '')
  if (s.length === 3)
    s = s
      .split('')
      .map(c => c + c)
      .join('')
  return [parseInt(s.slice(0, 2), 16), parseInt(s.slice(2, 4), 16), parseInt(s.slice(4, 6), 16)]
}

const mix = (a: string, b: string, t: number): string => {
  const A = h2r(a)
  const B = h2r(b)
  return (
    '#' +
    A.map((v, i) =>
      Math.round(v + (B[i] - v) * t)
        .toString(16)
        .padStart(2, '0')
    ).join('')
  )
}

/** Convert an `"H S% L%"` triplet (as used by the brand palette) to hex. */
export const hslToHex = (hsl: string): string => {
  const [h, s, l] = hsl.replace(/%/g, '').trim().split(/\s+/).map(Number)
  const a = (s / 100) * Math.min(l / 100, 1 - l / 100)
  const f = (n: number) => {
    const k = (n + h / 30) % 12
    const c = l / 100 - a * Math.max(Math.min(k - 3, 9 - k, 1), -1)
    return Math.round(255 * c)
      .toString(16)
      .padStart(2, '0')
  }
  return `#${f(0)}${f(8)}${f(4)}`
}

const relLuminance = (hex: string): number => {
  const lin = (c: number) => {
    const s = c / 255
    return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4)
  }
  const [r, g, b] = h2r(hex)
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

const contrastRatio = (a: string, b: string): number => {
  const la = relLuminance(a)
  const lb = relLuminance(b)
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05)
}

const ACCENT_INK = '#0D1117'

/** Readable text/icon color to lay on top of a solid accent fill (max contrast). */
export const onAccent = (accent: string): string =>
  contrastRatio(accent, ACCENT_INK) >= contrastRatio(accent, '#FFFFFF') ? ACCENT_INK : '#FFFFFF'

export interface PortalBrandPreset {
  key: string
  label: string
  /** Accent hex tuned for each mode (light = saturated, dark = pastel). */
  light: string
  dark: string
}

/**
 * Curated accent palette offered in portal settings. Each entry carries a hex
 * tuned per mode; only the variant matching the active theme is stored/shown,
 * so swapping mode simply re-surfaces that mode's variants (a value that no
 * longer matches falls through to "custom").
 */
export const BRAND_PRESETS: PortalBrandPreset[] = (
  [
    { key: 'hex', label: 'Hex violet', light: '255 38% 37%', dark: '0 71% 86%' },
    { key: 'indigo', label: 'Linear indigo', light: '239 84% 60%', dark: '234 89% 74%' },
    { key: 'emerald', label: 'Emerald', light: '160 84% 32%', dark: '158 64% 60%' },
    { key: 'amber', label: 'Amber', light: '32 95% 44%', dark: '38 92% 65%' },
    { key: 'rose', label: 'Rose', light: '346 77% 50%', dark: '350 89% 75%' },
    { key: 'ocean', label: 'Ocean', light: '201 96% 32%', dark: '199 89% 72%' },
    { key: 'sage', label: 'Sage', light: '145 18% 38%', dark: '140 18% 72%' },
    { key: 'dusty-teal', label: 'Dusty teal', light: '186 25% 38%', dark: '186 28% 72%' },
    { key: 'taupe', label: 'Warm taupe', light: '28 22% 42%', dark: '28 28% 78%' },
    { key: 'mauve', label: 'Mauve', light: '320 14% 44%', dark: '320 20% 80%' },
    { key: 'clay', label: 'Clay', light: '12 32% 48%', dark: '14 38% 76%' },
    { key: 'slate-neutral', label: 'Slate', light: '215 14% 34%', dark: '215 16% 78%' },
    { key: 'graphite', label: 'Graphite', light: '0 0% 16%', dark: '0 0% 92%' },
    { key: 'mint', label: 'Mint', light: '152 38% 42%', dark: '152 48% 74%' },
    { key: 'seafoam', label: 'Seafoam', light: '168 34% 40%', dark: '168 40% 74%' },
    { key: 'pistachio', label: 'Pistachio', light: '82 26% 44%', dark: '82 32% 74%' },
    { key: 'sky', label: 'Sky', light: '205 58% 46%', dark: '205 70% 76%' },
    { key: 'lavender', label: 'Lavender', light: '258 28% 54%', dark: '258 42% 80%' },
  ] as const
).map(b => ({ key: b.key, label: b.label, light: hslToHex(b.light), dark: hslToHex(b.dark) }))

/** Curated accents for a given theme mode, as hex (for swatches/storage). */
export const brandPresetsFor = (mode: PortalThemeMode): { label: string; hex: string }[] =>
  BRAND_PRESETS.map(b => ({ label: b.label, hex: b[mode] }))

type Tokens = Record<string, string>

/** Color tokens for a given accent + mode. Mirrors the design `tokens()`. */
export const colorTokens = (accent: string, mode: PortalThemeMode): Tokens => {
  const inkL = mix(accent, '#000000', 0.46)
  const weakL = mix(accent, '#FFFFFF', 0.85)
  const weakD = mix(accent, '#0A0A0A', 0.85)

  if (mode === 'dark') {
    return {
      '--mtp-accent': accent,
      '--mtp-accent-ink': accent,
      '--mtp-on-accent': onAccent(accent),
      '--mtp-accent-weak': weakD,
      '--mtp-bg': '#0A0A0A',
      '--mtp-surface': '#151515',
      '--mtp-surface-2': '#1E1E1E',
      '--mtp-border': 'rgba(255,255,255,0.10)',
      '--mtp-border-2': 'rgba(255,255,255,0.16)',
      '--mtp-text': '#F5F5F5',
      '--mtp-text-2': '#9A9A9A',
      '--mtp-text-3': '#6A6A6A',
      '--mtp-track': 'rgba(255,255,255,0.08)',
      '--mtp-fill': accent,
      // Spotlight: a brand-tinted elevated dark surface (not a jarring white
      // block) so it reads as part of the dark system, with the accent on fills.
      '--mtp-spot': mix(accent, '#101012', 0.9),
      '--mtp-spot-border': 'rgba(255,255,255,0.08)',
      '--mtp-spot-text': '#F5F5F5',
      '--mtp-spot-2': '#A1A1AA',
      '--mtp-spot-3': '#7A7A82',
      '--mtp-spot-track': 'rgba(255,255,255,0.12)',
      '--mtp-spot-fill': accent,
      // Semantic status tones — decoupled from the brand accent and tuned for
      // dark backgrounds so tags stay legible on any accent.
      '--mtp-ok-text': '#4ADE80',
      '--mtp-ok-bg': 'rgba(34,197,94,0.15)',
      '--mtp-ok-dot': '#22C55E',
      '--mtp-warn-text': '#FBBF24',
      '--mtp-warn-bg': 'rgba(245,158,11,0.16)',
      '--mtp-danger': '#FB7185',
      '--mtp-danger-bg': 'rgba(251,113,133,0.16)',
      '--mtp-header-bg': 'rgba(10,10,10,0.82)',
      '--mtp-overlay': 'rgba(0,0,0,0.6)',
      '--mtp-sheet': '#0F0F0F',
    }
  }
  return {
    '--mtp-accent': accent,
    '--mtp-accent-ink': inkL,
    '--mtp-on-accent': onAccent(accent),
    '--mtp-accent-weak': weakL,
    '--mtp-bg': '#F5F5F6',
    '--mtp-surface': '#FFFFFF',
    '--mtp-surface-2': '#F1F1F3',
    '--mtp-border': 'rgba(0,0,0,0.08)',
    '--mtp-border-2': 'rgba(0,0,0,0.13)',
    '--mtp-text': '#18181B',
    '--mtp-text-2': '#62626C',
    '--mtp-text-3': '#9C9CA6',
    '--mtp-track': 'rgba(0,0,0,0.06)',
    '--mtp-fill': inkL,
    // Spotlight in light mode: a soft accent-tinted card with dark text, rather
    // than a heavy near-black block that fights the rest of the light surface.
    '--mtp-spot': mix(accent, '#FFFFFF', 0.82),
    '--mtp-spot-border': mix(accent, '#FFFFFF', 0.5),
    '--mtp-spot-text': '#18181B',
    '--mtp-spot-2': '#62626C',
    '--mtp-spot-3': inkL,
    '--mtp-spot-track': 'rgba(0,0,0,0.06)',
    '--mtp-spot-fill': accent,
    '--mtp-ok-text': '#15803D',
    '--mtp-ok-bg': 'rgba(34,197,94,0.12)',
    '--mtp-ok-dot': '#16A34A',
    '--mtp-warn-text': '#B45309',
    '--mtp-warn-bg': 'rgba(245,158,11,0.14)',
    '--mtp-danger': '#DC2626',
    '--mtp-danger-bg': 'rgba(220,38,38,0.10)',
    '--mtp-header-bg': 'rgba(245,245,246,0.82)',
    '--mtp-overlay': 'rgba(20,20,22,0.4)',
    '--mtp-sheet': '#FFFFFF',
  }
}

const RADII: Record<PortalRoundness, [string, string, string]> = {
  Sharp: ['4px', '4px', '3px'],
  Modern: ['10px', '8px', '6px'],
  Rounded: ['18px', '13px', '9px'],
}

export const radiusTokens = (roundness: PortalRoundness): Tokens => {
  const r = RADII[roundness] ?? RADII.Modern
  return { '--mtp-r-card': r[0], '--mtp-r-ctrl': r[1], '--mtp-r-sm': r[2] }
}

const FONT_TOKENS: Tokens = {
  '--mtp-font': "'Geist', 'Inter var', 'Inter', system-ui, -apple-system, sans-serif",
  '--mtp-mono': "'Geist Mono', 'JetBrains Mono', ui-monospace, monospace",
}

/** Build the full CSS-variable map for a resolved config. */
export const buildTokens = (cfg: PortalThemeConfig): Tokens => {
  const tokens: Tokens = {
    ...colorTokens(cfg.accent, cfg.theme),
    ...radiusTokens(cfg.roundness),
    ...FONT_TOKENS,
  }
  // Curated host overrides win over the accent-derived palette.
  if (cfg.colors) {
    for (const key of Object.keys(TOKEN_BY_COLOR) as (keyof PortalColorOverrides)[]) {
      const value = cfg.colors[key]
      if (value) tokens[TOKEN_BY_COLOR[key]] = value
    }
  }
  return tokens
}

const oneOf = <T extends string>(v: string | null, options: readonly T[]): T | undefined =>
  v && (options as readonly string[]).includes(v) ? (v as T) : undefined

/**
 * Resolve the active theme from URL overrides, tenant branding and defaults.
 * `search` defaults to the current location so embeds can pass overrides via
 * the iframe URL.
 */
export const resolveTheme = (
  branding?: PortalBranding,
  search: string = typeof window !== 'undefined' ? window.location.search : ''
): PortalThemeConfig => {
  const p = new URLSearchParams(search)

  const urlAccent = p.get('accent')
  const accent =
    (isHex(urlAccent) ? urlAccent : undefined) ??
    (isHex(branding?.accent) ? branding!.accent : undefined) ??
    DEFAULT_THEME.accent

  const theme =
    oneOf(p.get('theme'), ['light', 'dark'] as const) ?? branding?.theme ?? DEFAULT_THEME.theme

  const roundness =
    oneOf(p.get('radius'), ['Sharp', 'Modern', 'Rounded'] as const) ??
    branding?.roundness ??
    DEFAULT_THEME.roundness

  return { theme, roundness, accent, colors: readColorOverrides(p) }
}

/** Read the curated per-token color overrides (hex only) from the URL. */
const readColorOverrides = (p: URLSearchParams): PortalColorOverrides | undefined => {
  const out: PortalColorOverrides = {}
  for (const key of Object.keys(TOKEN_BY_COLOR) as (keyof PortalColorOverrides)[]) {
    const v = p.get(key)
    if (isHex(v)) out[key] = v
  }
  return Object.keys(out).length ? out : undefined
}

/**
 * Resolve whether the "Powered by Meteroid" footer shows. On by default; the
 * tenant can turn it off in branding, and the embed SDK can force it off per
 * iframe via `?branding=false` (the SDK's `branding: false` option).
 */
export const resolvePoweredBy = (
  branding?: PortalBranding,
  search: string = typeof window !== 'undefined' ? window.location.search : ''
): boolean => {
  if (new URLSearchParams(search).get('branding') === 'false') return false
  return branding?.showPoweredBy !== false
}
