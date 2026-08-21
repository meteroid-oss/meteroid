import { buildTokens } from '@/pages/portal/experience/theme'

import './checkout-theme.css'

import type { PortalThemeConfig } from '@/pages/portal/experience/theme'
import type { CSSProperties, ReactNode } from 'react'

interface CheckoutThemePaneProps {
  config: PortalThemeConfig
  className?: string
  children: ReactNode
}

/**
 * Scoped theme wrapper for the RIGHT (interactive) panel of the checkout /
 * invoice-payment split layouts. Sets the portal `--mtp-*` variables on a
 * single root (mirroring PortalThemeProvider) so the interactive half follows
 * the resolved theme while the left summary stays light.
 *
 * In light mode this paints white/light surfaces (visually identical to the
 * pre-theming look); in dark mode it switches to the portal dark tokens.
 */
export const CheckoutThemePane = ({ config, className, children }: CheckoutThemePaneProps) => {
  const tokenStyle = buildTokens(config) as unknown as CSSProperties

  return (
    <div
      data-mtp-theme={config.theme}
      className={`mtp-checkout-pane ${className ?? ''}`}
      style={{
        ...tokenStyle,
        background: 'var(--mtp-bg)',
        color: 'var(--mtp-text)',
      }}
    >
      {children}
    </div>
  )
}
