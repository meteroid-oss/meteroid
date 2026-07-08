import { createContext, useContext, useMemo } from 'react'

import {
  buildTokens,
  DEFAULT_THEME,
  PortalBranding,
  PortalRoundness,
  PortalThemeConfig,
  PortalThemeMode,
  resolvePoweredBy,
  resolveTheme,
} from './theme'

import type { CSSProperties, ReactNode } from 'react'

export interface PortalConfig extends PortalThemeConfig {
  branding: PortalBranding
}

const PortalConfigContext = createContext<PortalConfig>({
  ...DEFAULT_THEME,
  branding: {},
})

export const usePortalConfig = () => useContext(PortalConfigContext)
export const usePortalTokenStyle = (): CSSProperties =>
  buildTokens(usePortalConfig()) as unknown as CSSProperties

interface ProviderProps {
  branding?: PortalBranding
  /** Force a theme regardless of URL/branding — used by embed previews. */
  forceTheme?: PortalThemeMode
  forceRoundness?: PortalRoundness
  forceAccent?: string
  search?: string
  /** When true, the provider does not paint the page background (embed mode). */
  bare?: boolean
  className?: string
  style?: CSSProperties
  children: ReactNode
}

/**
 * Establishes the scoped portal theme. All portal CSS variables live on the
 * root node this renders, isolating the portal from the host app's tokens and
 * giving embeds a self-contained, themeable surface.
 */
export const PortalThemeProvider = ({
  branding = {},
  forceTheme,
  forceRoundness,
  forceAccent,
  search,
  bare = false,
  className,
  style,
  children,
}: ProviderProps) => {
  const config = useMemo<PortalConfig>(() => {
    const resolved = resolveTheme(branding, search)
    return {
      theme: forceTheme ?? resolved.theme,
      roundness: forceRoundness ?? resolved.roundness,
      accent: forceAccent ?? resolved.accent,
      colors: resolved.colors,
      // Fold the per-iframe `?branding=false` override into the branding the
      // footer reads, so portal and compact widgets share one gate.
      branding: { ...branding, showPoweredBy: resolvePoweredBy(branding, search) },
    }
  }, [branding, forceTheme, forceRoundness, forceAccent, search])

  const tokenStyle = useMemo(() => buildTokens(config) as unknown as CSSProperties, [config])

  return (
    <PortalConfigContext.Provider value={config}>
      <div
        className={className}
        data-mtp-theme={config.theme}
        style={{
          ...tokenStyle,
          color: 'var(--mtp-text)',
          background: bare ? 'transparent' : 'var(--mtp-bg)',
          fontFamily: 'var(--mtp-font)',
          fontSize: '14px',
          letterSpacing: '-0.005em',
          WebkitFontSmoothing: 'antialiased',
          minHeight: bare ? undefined : '100vh',
          ...style,
        }}
      >
        {children}
      </div>
    </PortalConfigContext.Provider>
  )
}
