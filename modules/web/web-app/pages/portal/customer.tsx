import { useEffect } from 'react'

import { useForceTheme } from 'providers/ThemeProvider'

import { PortalApp } from './experience/PortalApp'
import { PortalThemeProvider } from './experience/PortalThemeProvider'
import { EmbedHost, getEmbedView, type EmbedView } from './experience/embed/EmbedHost'
import { usePortalOverview } from './experience/hooks'
import { resolveTheme } from './experience/theme'

/**
 * Customer billing portal entry point.
 *
 * Renders the scoped, themeable portal experience. Tenant branding (accent /
 * logo / name / theme) is read from the customer overview (invoicing-entity
 * settings) and merged with any `?theme/accent/radius` URL overrides by the
 * theme provider. The global app theme is forced to match the resolved portal
 * theme so the few reused shared components (Stripe payment dialog, billing
 * form) stay consistent.
 *
 * When the URL carries `?embed=<view>` the portal renders a chromeless,
 * auto-resizing widget for iframe embedding instead of the full app.
 */
export const PortalCustomer = () => {
  const embedView = getEmbedView()

  if (embedView) {
    return <PortalEmbed view={embedView} />
  }

  return <PortalFull />
}

const PortalFull = () => {
  const { branding } = usePortalOverview()
  useForceTheme(resolveTheme(branding).theme)
  return (
    <PortalThemeProvider className="mtp" branding={branding}>
      <PortalApp />
    </PortalThemeProvider>
  )
}

const PortalEmbed = ({ view }: { view: EmbedView }) => {
  const { branding } = usePortalOverview()
  useForceTheme(resolveTheme(branding).theme)
  useTransparentBody()
  return (
    <PortalThemeProvider className="mtp" branding={branding} bare>
      <EmbedHost view={view} />
    </PortalThemeProvider>
  )
}

/**
 * Make the document see-through in embed mode so the iframe blends into the host
 * page (the widget cards keep their own surface). Toggles a marker class that a
 * global stylesheet rule keys off — it clears both the `bg-background` and the
 * `color-scheme: dark` canvas the browser would otherwise paint behind the
 * transparent portal root. A class (not inline styles) is used so it survives
 * ThemeProvider re-applying the inline `colorScheme` on theme changes.
 */
const useTransparentBody = () => {
  useEffect(() => {
    document.documentElement.classList.add('mtp-embed')
    return () => document.documentElement.classList.remove('mtp-embed')
  }, [])
}
