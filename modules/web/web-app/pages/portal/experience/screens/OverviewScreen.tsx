import { Slot } from '@/edition'
import { useQuery } from '@/lib/connectrpc'
import { listAvailablePlans } from '@/rpc/portal/subscription/v1/subscription-PortalSubscriptionService_connectquery'

import { usePortalNav } from '../context'
import { date, money, nextPayment, pmLabel, subStatusBadge } from '../format'
import { useCreditOverview, useUsageLimits } from '../hooks'
import { planLimitOf } from '../limits'
import {
  PAY_AS_YOU_GO,
  isFreePlan,
  isPayAsYouGo,
  perCadence,
  planPriceAmount,
  planPriceLines,
  usageRatesLine,
} from '../planPrice'
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

import { PortalAlerts } from './AlertBanner'
import { CreditSummary } from './CreditCards'
import { LimitsCard, PlanLimitCard } from './LimitsCard'
import { PlanPriceHeadline } from './PlanPriceHeadline'
import { RecentInvoices } from './RecentInvoices'
import { UPGRADE_SECTION_ID, UpgradeSection } from './UpgradeSection'
import { UsageFilterTag } from './UsageFilterTag'

import type { CustomerPaymentMethod } from '@/rpc/api/customers/v1/models_pb'
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

const CardIcon = () => (
  <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="var(--mtp-text-3)" strokeWidth="1.7">
    <rect x="2" y="5" width="20" height="14" rx="2.5" />
    <path d="M2 10h20" />
  </svg>
)

const PaymentMethodRow = ({ pm, onChange }: { pm: CustomerPaymentMethod; onChange: () => void }) => (
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
    <CardIcon />
    <span style={{ fontSize: 12.5, fontWeight: 500 }}>{pmLabel(pm)}</span>
    <LinkButton style={{ marginLeft: 'auto', fontSize: 12 }} onClick={onChange}>
      Change
    </LinkButton>
  </div>
)

const SubscriptionPrice = ({ sub }: { sub: SubscriptionSummary }) => {
  if (!sub.planPrice) return <div style={{ marginLeft: 'auto' }} />
  const payg = isPayAsYouGo(sub.planPrice)
  return (
    <div style={{ marginLeft: 'auto', display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: 2 }}>
      {payg ? (
        <span style={{ fontSize: 13.5, fontWeight: 500, color: 'var(--mtp-text)' }}>{PAY_AS_YOU_GO}</span>
      ) : (
        <Mono style={{ fontSize: 13.5, fontWeight: 500, color: 'var(--mtp-text)' }}>
          {planPriceAmount(sub.planPrice)}
        </Mono>
      )}
      <span style={{ fontSize: 11, color: 'var(--mtp-text-3)' }}>
        {payg ? usageRatesLine(sub.planPrice.usageRates!) : perCadence(sub.planPrice)}
      </span>
    </div>
  )
}

const scrollTo = (id: string) =>
  document.getElementById(id)?.scrollIntoView({ behavior: 'smooth', block: 'start' })

export const OverviewScreen = () => {
  const { overview, goTo, openSubscription } = usePortalNav()
  const subs = overview.activeSubscriptions
  const primary = subs[0] as SubscriptionSummary | undefined

  // The credit overview carries each subscription's next invoice, computed once for the page.
  const credit = useCreditOverview()
  const limitsQuery = useUsageLimits()
  const limits = limitsQuery.data?.limits ?? []
  const planLimit = planLimitOf(limits)
  const plans = useQuery(
    listAvailablePlans,
    { subscriptionId: primary?.id ?? '' },
    { enabled: !!primary && !!planLimit && (planLimit.reached || planLimit.warning) }
  )
  const upgradePlans = (plans.data?.plans ?? []).filter(plan => !plan.isCurrent)

  const nextInvoice = credit.data?.upcomingInvoices.find(
    invoice => invoice.subscriptionId === primary?.id
  )
  const nextLoading = credit.isLoading
  const nextUnavailable =
    credit.isError ||
    (!!primary && !!credit.data?.unavailableUpcomingSubscriptionIds.includes(primary.id))
  const payment = nextInvoice ? nextPayment(nextInvoice) : undefined
  const defaultPm =
    overview.paymentMethods.find(p => p.id === overview.customer?.currentPaymentMethodId) ??
    overview.paymentMethods[0]
  const price = primary?.planPrice
  const freePlan = !!price && isFreePlan(price)
  const showPlanLimit =
    !!planLimit &&
    freePlan &&
    !nextLoading &&
    !nextUnavailable &&
    (nextInvoice?.amountDue ?? 0n) === 0n
  const otherLimits = limits.filter(limit => !(showPlanLimit && limit === planLimit))
  const showUpgrade =
    !!primary && !!planLimit && (planLimit.reached || planLimit.warning) && upgradePlans.length > 0
  const autoChargePm = payment?.paymentMethod

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <PortalAlerts
        credit={credit.data}
        limits={limits}
        freePlan={freePlan}
        onAddCredit={() => goTo('balance')}
        onUpgrade={showUpgrade ? () => scrollTo(UPGRADE_SECTION_ID) : undefined}
      />

      <p style={{ fontSize: 14, color: 'var(--mtp-text-2)', margin: 0 }}>
        {subs.length} active subscription{subs.length === 1 ? '' : 's'}
        {payment
          ? ` · ${payment.charged ? 'next charge' : 'next payment due'} ${date(payment.date)}`
          : primary?.nextBillingDate
            ? ` · renews ${date(primary.nextBillingDate)}`
            : ''}
      </p>

      {primary && (
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: 'minmax(0,1.3fr) minmax(0,1fr)',
            gap: 16,
          }}
          className="mtp-overview-grid"
        >
          <SpotlightCard style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
            <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between' }}>
              <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                <Eyebrow spot>Current plan</Eyebrow>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                  <span style={{ fontSize: 22, fontWeight: 600, letterSpacing: '-0.02em' }}>
                    {primary.planName}
                  </span>
                  {primary.usageFilter && (
                    <Mono style={{ fontSize: 12, color: 'var(--mtp-text-2)' }}>
                      {primary.usageFilter.property} = {primary.usageFilter.value}
                    </Mono>
                  )}
                  <Pill badge={subStatusBadge(primary.status)} dot />
                </div>
              </div>
              <PButton size="sm" onClick={() => openSubscription(primary.id)}>
                Manage
              </PButton>
            </div>
            {price && <PlanPriceHeadline price={price} size={34} suffixColor="var(--mtp-spot-2)" />}
            {(price ? planPriceLines(price, primary.planDescription) : [primary.planDescription])
              .filter(Boolean)
              .map(line => (
                <span key={line} style={{ fontSize: 14 }}>
                  {line}
                </span>
              ))}
            {primary.pendingCancellationDate && (
              <span style={{ fontSize: 12.5, color: 'var(--mtp-danger)' }}>
                Cancels {date(primary.pendingCancellationDate)}
              </span>
            )}
          </SpotlightCard>

          {showPlanLimit && planLimit ? (
            <PlanLimitCard limit={planLimit} />
          ) : (
            <Card pad={24} style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
              <Eyebrow>Next payment</Eyebrow>
              {nextInvoice ? (
                <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                  <Mono style={{ fontSize: 32, fontWeight: 600, letterSpacing: '-0.03em' }}>
                    {nextInvoice.creditEstimateShared && '≈ '}
                    {money(nextInvoice.amountDue, nextInvoice.currency)}
                  </Mono>
                  <span style={{ fontSize: 13.5 }}>
                    {payment?.charged
                      ? `Charged automatically on ${date(payment.date)}`
                      : `Due ${date(payment?.date)}`}
                  </span>
                </div>
              ) : (
                <span style={{ fontSize: 13.5, color: 'var(--mtp-text-2)' }}>
                  {nextLoading
                    ? '—'
                    : nextUnavailable
                      ? 'Unavailable right now. Try again in a few minutes.'
                      : 'No upcoming charge'}
                </span>
              )}
              {autoChargePm ? (
                <PaymentMethodRow pm={autoChargePm} onChange={() => goTo('settings')} />
              ) : (
                defaultPm && <PaymentMethodRow pm={defaultPm} onChange={() => goTo('settings')} />
              )}
            </Card>
          )}
        </div>
      )}

      <CreditSummary
        credit={credit.data}
        limitsCard={otherLimits.length > 0 ? <LimitsCard limits={otherLimits} /> : null}
      />

      {showUpgrade && primary && (
        <UpgradeSection plans={upgradePlans} subscriptionId={primary.id} currency={primary.currency} />
      )}

      {subs.length !== 1 && (
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
                <span
                  style={{
                    display: 'inline-flex',
                    alignItems: 'baseline',
                    gap: 8,
                    fontSize: 13.5,
                    fontWeight: 500,
                    color: 'var(--mtp-text)',
                  }}
                >
                  {s.planName}
                  <UsageFilterTag subscriptionId={s.id} />
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
              <SubscriptionPrice sub={s} />
              <Chevron />
            </button>
          ))}
          {subs.length === 0 && (
            <div style={{ padding: '24px 20px', fontSize: 13, color: 'var(--mtp-text-2)' }}>
              No active subscriptions.
            </div>
          )}
        </PanelCard>
      )}

      <RecentInvoices invoices={overview.recentInvoices} />

      <Slot name="portal.overview.sections" />
    </div>
  )
}
