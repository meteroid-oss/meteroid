import type { PortalThemeConfig, PortalRoundness } from '@/pages/portal/experience/theme'
import type { Appearance } from '@stripe/stripe-js'

/** Stripe `borderRadius` per portal roundness, mirroring the control radii. */
const RADIUS_BY_ROUNDNESS: Record<PortalRoundness, string> = {
  Sharp: '4px',
  Modern: '8px',
  Rounded: '18px',
}

/**
 * Build a Stripe Elements `Appearance` from the resolved portal theme so the
 * embedded payment fields match the surrounding (right-panel) surface.
 *
 * Stripe applies `appearance` at Elements creation; since the portal theme is
 * fixed per page load (URL param), passing this once at `<Elements options>` is
 * sufficient (no `elements.update` needed).
 */
export const buildStripeAppearance = (cfg: PortalThemeConfig): Appearance => {
  const isDark = cfg.theme === 'dark'

  return {
    theme: isDark ? 'night' : 'stripe',
    variables: {
      colorPrimary: cfg.accent,
      colorBackground: isDark ? '#151515' : '#FFFFFF',
      colorText: isDark ? '#F5F5F5' : '#18181B',
      colorTextSecondary: isDark ? '#9A9A9A' : '#62626C',
      colorDanger: isDark ? '#FB7185' : '#DC2626',
      fontFamily: 'Inter, sans-serif',
      fontSizeBase: '14px',
      borderRadius: RADIUS_BY_ROUNDNESS[cfg.roundness] ?? RADIUS_BY_ROUNDNESS.Modern,
      gridRowSpacing: '1rem',
    },
  }
}
