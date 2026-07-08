import { usePortalNav } from '../context'
import { date, money, subStatusBadge } from '../format'
import { Card, LinkButton, Mono, PButton, Pill } from '../primitives'

const SubGlyph = ({ size = 44 }: { size?: number }) => (
  <div
    style={{
      width: size,
      height: size,
      borderRadius: 'var(--mtp-r-ctrl)',
      background: 'var(--mtp-accent-weak)',
      color: 'var(--mtp-accent-ink)',
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      flex: `0 0 ${size}px`,
    }}
  >
    <svg width={size * 0.45} height={size * 0.45} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.9">
      <path d="M4 7h16M4 12h16M4 17h10" />
    </svg>
  </div>
)

export const SubscriptionsScreen = () => {
  const { overview, openSubscription } = usePortalNav()
  const subs = overview.activeSubscriptions

  if (subs.length === 0) {
    return (
      <Card>
        <span style={{ fontSize: 13, color: 'var(--mtp-text-2)' }}>No active subscriptions.</span>
      </Card>
    )
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      {subs.map(s => (
        <Card key={s.id} style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
            <SubGlyph />
            <div style={{ display: 'flex', flexDirection: 'column', gap: 4, minWidth: 0, flex: 1 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <span style={{ fontSize: 16, fontWeight: 600, letterSpacing: '-0.01em' }}>
                  {s.planName}
                </span>
                <Pill badge={subStatusBadge(s.status)} dot />
              </div>
              <span style={{ fontSize: 13, color: 'var(--mtp-text-2)' }}>
                {s.pendingCancellationDate ? (
                  <span style={{ color: 'var(--mtp-danger)' }}>
                    cancels {date(s.pendingCancellationDate)}
                  </span>
                ) : s.nextBillingDate ? (
                  `renews ${date(s.nextBillingDate)}`
                ) : (
                  subStatusBadge(s.status).label
                )}
              </span>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: 2 }}>
              <Mono style={{ fontSize: 20, fontWeight: 600, letterSpacing: '-0.02em' }}>
                {money(s.mrrCents, s.currency)}
              </Mono>
              <span style={{ fontSize: 11.5, color: 'var(--mtp-text-3)' }}>per month</span>
            </div>
          </div>
          <div
            style={{
              display: 'flex',
              gap: 9,
              paddingTop: 14,
              borderTop: '1px solid var(--mtp-border)',
            }}
          >
            <PButton onClick={() => openSubscription(s.id)}>Manage</PButton>
            <LinkButton style={{ marginLeft: 'auto' }} onClick={() => openSubscription(s.id)}>
              View details
            </LinkButton>
          </div>
        </Card>
      ))}
    </div>
  )
}
