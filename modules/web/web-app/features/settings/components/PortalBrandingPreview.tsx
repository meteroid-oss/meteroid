import { PortalThemeProvider } from '@/pages/portal/experience/PortalThemeProvider'
import {
  Card,
  Eyebrow,
  Meter,
  Mono,
  PButton,
  Pill,
} from '@/pages/portal/experience/primitives'
import {
  DEFAULT_ACCENT,
  PortalRoundness,
  PortalThemeMode,
} from '@/pages/portal/experience/theme'

interface PortalBrandingPreviewProps {
  accent?: string
  theme?: string
  roundness?: string
}

const HEX = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i

const asTheme = (v?: string): PortalThemeMode => (v === 'dark' ? 'dark' : 'light')
const asRoundness = (v?: string): PortalRoundness =>
  v === 'Sharp' || v === 'Rounded' ? v : 'Modern'

/**
 * Static, self-contained replica of the embedded "current plan" widget, skinned
 * live from the branding form values. Uses mock data so it needs no portal
 * session — it exists purely to show how the chosen accent reads in context.
 */
export const PortalBrandingPreview = ({ accent, theme, roundness }: PortalBrandingPreviewProps) => {
  const effectiveAccent = accent && HEX.test(accent) ? accent : DEFAULT_ACCENT

  return (
    <div className="flex flex-col gap-2">
      <span className="text-xs text-muted-foreground">Embed preview</span>
      <PortalThemeProvider
        bare
        className="mtp"
        forceAccent={effectiveAccent}
        forceTheme={asTheme(theme)}
        forceRoundness={asRoundness(roundness)}
        style={{
          borderRadius: 12,
          padding: 20,
          background: 'var(--mtp-bg)',
          border: '1px solid var(--mtp-border)',
        }}
      >
        <Card pad={20} style={{ display: 'flex', flexDirection: 'column', gap: 16, maxWidth: 340 }}>
          <div
            style={{
              display: 'flex',
              alignItems: 'flex-start',
              justifyContent: 'space-between',
              gap: 12,
            }}
          >
            <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              <Eyebrow>Current plan</Eyebrow>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <span style={{ fontSize: 18, fontWeight: 600, letterSpacing: '-0.02em' }}>
                  Scale
                </span>
                <Pill badge={{ label: 'Active', tone: 'ok' }} dot />
              </div>
            </div>
            <PButton size="sm">Manage</PButton>
          </div>

          <div style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
            <Mono style={{ fontSize: 26, fontWeight: 600, letterSpacing: '-0.03em' }}>$490.00</Mono>
            <span style={{ fontSize: 13, color: 'var(--mtp-text-2)' }}>/ mo</span>
          </div>

          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            <div
              style={{
                display: 'flex',
                justifyContent: 'space-between',
                fontSize: 12.5,
                color: 'var(--mtp-text-2)',
              }}
            >
              <span>API requests</span>
              <span style={{ color: 'var(--mtp-text)' }}>720k / 1M</span>
            </div>
            <Meter pct={72} />
          </div>
        </Card>
      </PortalThemeProvider>
    </div>
  )
}
