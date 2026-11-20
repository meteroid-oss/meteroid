import {
  PortalBranding,
  PortalRoundness,
  PortalThemeMode,
  onAccent,
  resolveTheme,
} from '@/pages/portal/experience/theme'
import { QuotePortalDetails } from '@/rpc/portal/quotes/v1/models_pb'

import type { CSSProperties } from 'react'

const asThemeMode = (v?: string): PortalThemeMode | undefined =>
  v === 'light' || v === 'dark' ? v : undefined
const asRoundness = (v?: string): PortalRoundness | undefined =>
  v === 'Sharp' || v === 'Modern' || v === 'Rounded' ? v : undefined

export const quoteBranding = (details?: QuotePortalDetails): PortalBranding => ({
  companyName: details?.entity?.legalName,
  logoUrl: details?.logoUrl,
  accent: details?.entity?.brandColor,
  theme: asThemeMode(details?.themeMode),
  roundness: asRoundness(details?.roundness),
  showPoweredBy: details?.showPoweredBy,
})

const hexToHslTriplet = (hex: string): string => {
  let s = hex.replace('#', '')
  if (s.length === 3)
    s = s
      .split('')
      .map(c => c + c)
      .join('')
  const [r, g, b] = [0, 2, 4].map(i => parseInt(s.slice(i, i + 2), 16) / 255)
  const max = Math.max(r, g, b)
  const min = Math.min(r, g, b)
  const l = (max + min) / 2
  const d = max - min
  if (d === 0) return `0 0% ${(l * 100).toFixed(1)}%`
  const sat = d / (1 - Math.abs(2 * l - 1))
  const sector =
    max === r ? (g - b) / d + (g < b ? 6 : 0) : max === g ? (b - r) / d + 2 : (r - g) / d + 4
  const hue = sector * 60
  return `${hue.toFixed(1)} ${(sat * 100).toFixed(1)}% ${(l * 100).toFixed(1)}%`
}

const RADIUS: Record<PortalRoundness, string> = {
  Sharp: '0.25rem',
  Modern: '0.5rem',
  Rounded: '0.875rem',
}

/**
 * The seller's accent and roundness as the app's Tailwind tokens (`brand`, `ring`, `radius`).
 * Set on the portal root and on every overlay, which Radix renders outside of it.
 */
export const quoteBrandStyle = (details: QuotePortalDetails, ignoreUrl: boolean): CSSProperties => {
  const config = resolveTheme(quoteBranding(details), ignoreUrl ? '' : undefined)
  const brand = hexToHslTriplet(config.accent)
  return {
    '--brand': brand,
    '--brand-foreground': hexToHslTriplet(onAccent(config.accent)),
    '--ring': brand,
    '--radius': RADIUS[config.roundness] ?? RADIUS.Modern,
  } as CSSProperties
}
