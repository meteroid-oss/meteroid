import { resolveTheme } from '@/pages/portal/experience/theme'

import type {
  PortalBranding,
  PortalRoundness,
  PortalThemeConfig,
  PortalThemeMode,
} from '@/pages/portal/experience/theme'

/** Invoicing-entity branding/theme fields carried on the checkout & invoice-payment payloads. */
export interface CheckoutBrandingFields {
  brandColor?: string
  themeMode?: string
  roundness?: string
  logoUrl?: string
  tradeName?: string
}

const asThemeMode = (v?: string): PortalThemeMode | undefined =>
  v === 'light' || v === 'dark' ? v : undefined
const asRoundness = (v?: string): PortalRoundness | undefined =>
  v === 'Sharp' || v === 'Modern' || v === 'Rounded' ? v : undefined

/**
 * Resolve the theme for the checkout / invoice-payment INTERACTIVE panel from
 * the invoicing-entity branding carried on the payload — the same DB source the
 * portal uses — so a payment page themes identically regardless of entry point.
 *
 * Precedence matches the portal (`resolveTheme`): URL overrides
 * (`?theme/accent/radius`, used by embeds/previews) win, then the entity
 * branding, then the built-in defaults.
 */
export const resolveCheckoutTheme = (
  fields?: CheckoutBrandingFields,
  search: string = typeof window !== 'undefined' ? window.location.search : ''
): PortalThemeConfig => {
  const branding: PortalBranding = {
    companyName: fields?.tradeName,
    logoUrl: fields?.logoUrl,
    accent: fields?.brandColor,
    theme: asThemeMode(fields?.themeMode),
    roundness: asRoundness(fields?.roundness),
  }
  return resolveTheme(branding, search)
}
