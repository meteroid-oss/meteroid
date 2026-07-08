import { useQuery } from '@/lib/connectrpc'
import { getUpcomingInvoice } from '@/rpc/portal/subscription/v1/subscription-PortalSubscriptionService_connectquery'

import { usePortalNav } from '../context'
import { money, date, pmLabel, subStatusBadge } from '../format'
import {
  Card,
  Eyebrow,
  LinkButton,
  PanelCard,
  PButton,
  Pill,
  SpotlightCard,
  Mono,
} from '../primitives'

import type { SubscriptionSummary } from '@/rpc/portal/customer/v1/models_pb'

const Chevron = () => (
  <svg
    width="16"
    height="16"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.8"
    style={{ color: 'var(--mtp-text-3)', flex: '0 0 16px' }}
  >
    <path d="M9 6l6 6-6 6" />
  </svg>
)

const SubIcon = ({ size = 36 }: { size?: number }) => (
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
    <svg width={size * 0.5} height={size * 0.5} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.9">
      <path d="M4 7h16M4 12h16M4 17h10" />
    </svg>
  </div>
)

export const OverviewScreen = () => {
  const { overview, goTo, openSubscription } = usePortalNav()
  const subs = overview.activeSubscriptions
  const primary = subs[0] as SubscriptionSummary | undefined

  // "Next payment" is the upcoming invoice for the primary subscription — NOT
  // the MRR. (MRR is the recurring plan price shown on the plan card.)
  const upcoming = useQuery(
    getUpcomingInvoice,
    { subscriptionId: primary?.id ?? '' },
    { enabled: !!primary }
  )
  const nextInvoice = upcoming.data?.invoice
  const nextDate = nextInvoice?.invoiceDate ?? primary?.nextBillingDate
  const canAddPayment = !!(overview.cardConnectionId || overview.directDebitConnectionId)
  const defaultPm =
    overview.paymentMethods.find(p => p.id === overview.customer?.currentPaymentMethodId) ??
    overview.paymentMethods[0]

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 22 }}>
      {/* Hero */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <p style={{ fontSize: 14, color: 'var(--mtp-text-2)', margin: 0 }}>
          {subs.length} active subscription{subs.length === 1 ? '' : 's'}
          {nextDate ? ` · next charge ${date(nextDate)}` : ''}
        </p>
      </div>

      {/* Plan + next payment */}
      {primary && (
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: 'minmax(0,1.3fr) minmax(0,1fr)',
            gap: 16,
          }}
          className="mtp-overview-grid"
        >
          <SpotlightCard style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
            <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between' }}>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                <Eyebrow spot>Current plan</Eyebrow>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                  <span style={{ fontSize: 22, fontWeight: 600, letterSpacing: '-0.02em' }}>
                    {primary.planName}
                  </span>
                  <Pill badge={subStatusBadge(primary.status)} dot />
                </div>
              </div>
              <PButton size="sm" onClick={() => openSubscription(primary.id)}>
                Manage
              </PButton>
            </div>
            <div style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
              <Mono style={{ fontSize: 34, fontWeight: 600, letterSpacing: '-0.03em' }}>
                {money(primary.mrrCents, primary.currency)}
              </Mono>
              <span style={{ fontSize: 14, color: 'var(--mtp-spot-2)' }}>/ month</span>
            </div>
            {primary.pendingCancellationDate && (
              <span style={{ fontSize: 12.5, color: 'var(--mtp-danger)' }}>
                Cancels {date(primary.pendingCancellationDate)}
              </span>
            )}
          </SpotlightCard>

          <Card pad={24} style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
            <Eyebrow>Next payment</Eyebrow>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
              <Mono style={{ fontSize: 32, fontWeight: 600, letterSpacing: '-0.03em', color: 'var(--mtp-text)' }}>
                {nextInvoice ? money(nextInvoice.amountDue, nextInvoice.currency) : '—'}
              </Mono>
              <span style={{ fontSize: 13, color: 'var(--mtp-text-2)' }}>
                {nextInvoice && nextDate ? `due ${date(nextDate)}` : 'no upcoming charge'}
              </span>
            </div>
            <div
              style={{
                marginTop: 'auto',
                display: 'flex',
                alignItems: 'center',
                gap: 10,
                padding: '11px 12px',
                borderRadius: 'var(--mtp-r-ctrl)',
                background: 'var(--mtp-surface-2)',
                border: '1px solid var(--mtp-border)',
              }}
            >
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="var(--mtp-text-3)" strokeWidth="1.7">
                <rect x="2" y="5" width="20" height="14" rx="2.5" />
                <path d="M2 10h20" />
              </svg>
              <span style={{ fontSize: 12.5, fontWeight: 500 }}>
                {defaultPm ? pmLabel(defaultPm) : 'No payment method'}
              </span>
              {/* Only offer Change/Add when the action is actually possible. */}
              {(defaultPm || canAddPayment) && (
                <LinkButton style={{ marginLeft: 'auto', fontSize: 12 }} onClick={() => goTo('settings')}>
                  {defaultPm ? 'Change' : 'Add'}
                </LinkButton>
              )}
            </div>
          </Card>
        </div>
      )}

      {/* Subscriptions list */}
      <PanelCard
        title="Subscriptions"
        action={<LinkButton onClick={() => goTo('subscriptions')}>View all</LinkButton>}
      >
        {subs.map((s, i) => (
          <button
            key={s.id}
            onClick={() => openSubscription(s.id)}
            className="mtp-hoverable"
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 12,
              padding: '14px 20px',
              borderTop: i === 0 ? 'none' : '1px solid var(--mtp-border)',
              width: '100%',
              background: 'transparent',
              border: 'none',
              cursor: 'pointer',
              fontFamily: 'inherit',
              textAlign: 'left',
            }}
          >
            <SubIcon />
            <div style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
              <span style={{ fontSize: 13.5, fontWeight: 500, color: 'var(--mtp-text)' }}>
                {s.planName}
              </span>
              <span style={{ fontSize: 12, color: 'var(--mtp-text-3)' }}>
                {s.pendingCancellationDate ? (
                  <span style={{ color: 'var(--mtp-danger)' }}>cancels {date(s.pendingCancellationDate)}</span>
                ) : s.nextBillingDate ? (
                  `renews ${date(s.nextBillingDate)}`
                ) : (
                  subStatusBadge(s.status).label
                )}
              </span>
            </div>
            <div
              style={{
                marginLeft: 'auto',
                display: 'flex',
                flexDirection: 'column',
                alignItems: 'flex-end',
                gap: 2,
              }}
            >
              <Mono style={{ fontSize: 13.5, fontWeight: 500, color: 'var(--mtp-text)' }}>
                {money(s.mrrCents, s.currency)}
              </Mono>
              <span style={{ fontSize: 11, color: 'var(--mtp-text-3)' }}>per month</span>
            </div>
            <Chevron />
          </button>
        ))}
        {subs.length === 0 && (
          <div style={{ padding: '24px 20px', fontSize: 13, color: 'var(--mtp-text-2)' }}>
            No active subscriptions.
          </div>
        )}
      </PanelCard>
    </div>
  )
}
