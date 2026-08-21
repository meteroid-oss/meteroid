import { useMutation } from '@connectrpc/connect-query'
import { useMemo, useState } from 'react'
import { toast } from 'sonner'

import { useQuery } from '@/lib/connectrpc'
import { AddPaymentMethodDialog } from '@/pages/portal/customer/AddPaymentMethodDialog'
import { InvoicePaymentStatus, InvoiceStatus } from '@/rpc/api/invoices/v1/models_pb'
import { setDefaultPaymentMethod } from '@/rpc/portal/shared/v1/shared-PortalSharedService_connectquery'
import { getSubscriptionDetails } from '@/rpc/portal/subscription/v1/subscription-PortalSubscriptionService_connectquery'
import { PendingEventType } from '@/rpc/portal/subscription/v1/subscription_pb'

import { BrandChip } from '../PaymentMethodChip'
import { PortalApp } from '../PortalApp'
import { usePortalConfig } from '../PortalThemeProvider'
import { date, feeLabel, money, pmLabel, pmSubLabel, subStatusBadge } from '../format'
import { useInvoices, usePortalOverview } from '../hooks'
import { Card, CenterState, Eyebrow, Mono, PanelCard, PButton, Pill, Spinner } from '../primitives'
import { UsageList } from '../screens/UsageScreen'

import { useAutoHeight } from './useAutoHeight'

import type { StatusBadge } from '../format'
import type {
  CustomerPortalOverview,
  InvoiceSummary,
  SubscriptionSummary,
} from '@/rpc/portal/customer/v1/models_pb'
import type { CSSProperties } from 'react'

/**
 * Embeddable, chromeless portal views.
 *
 * Loaded by the iframe when the URL carries `?embed=<view>`. The caller wraps
 * this in `<PortalThemeProvider bare>` so the scoped CSS variables are present;
 * `EmbedHost` itself renders only content (no theme provider, no page chrome).
 *
 * Each view auto-reports its height to the host SDK via `useAutoHeight`, and the
 * compact views can post a `meteroid:navigate` message so the host can open the
 * full portal.
 */

export type EmbedView =
  | 'portal'
  | 'plan'
  | 'subscriptions'
  | 'subscription'
  | 'usage'
  | 'invoices'
  | 'payment-methods'

const EMBED_VIEWS: readonly EmbedView[] = [
  'portal',
  'plan',
  'subscriptions',
  'subscription',
  'usage',
  'invoices',
  'payment-methods',
]

/** Parse the `subscription` id from the URL (for the single-subscription view). */
const getSubscriptionId = (): string | null => {
  if (typeof window === 'undefined') return null
  return new URLSearchParams(window.location.search).get('subscription')
}

/** Parse the `embed` view from a search string (defaults to current URL). */
export const getEmbedView = (
  search: string = typeof window !== 'undefined' ? window.location.search : ''
): EmbedView | null => {
  const v = new URLSearchParams(search).get('embed')
  return v && (EMBED_VIEWS as readonly string[]).includes(v) ? (v as EmbedView) : null
}

/**
 * Navigate to a portal page from a compact widget. `target` is a portal page
 * (`portal` for the overview, or a `?page=` value like `invoices`).
 *
 * When the host opted into handling navigation itself (SDK set `nav=host`
 * because `onNavigate` was provided), we post a message to the parent. Otherwise
 * we open the full hosted portal at the requested page in a new tab — which also
 * works when a raw `?embed=` URL is opened directly (no parent frame).
 */
const requestNavigate = (target: string) => {
  if (typeof window === 'undefined') return

  const current = new URLSearchParams(window.location.search)
  if (current.get('nav') === 'host' && window.parent !== window) {
    window.parent.postMessage({ type: 'meteroid:navigate', target }, '*')
    return
  }

  const url = new URL(window.location.href)
  url.searchParams.delete('embed')
  url.searchParams.delete('nav')
  url.searchParams.delete('count')
  if (target && target !== 'portal') url.searchParams.set('page', target)
  else url.searchParams.delete('page')
  window.open(url.toString(), '_blank', 'noopener')
}

/* ------------------------------------------------------------------- shells */

const LoadingShell = () => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      padding: '40px 24px',
      color: 'var(--mtp-text-3)',
    }}
  >
    <Spinner size={20} />
  </div>
)

const ErrorShell = () => (
  <Card>
    <CenterState
      title="Unavailable"
      hint="We couldn't load this billing widget. Your session may have expired."
    />
  </Card>
)

const Row = ({
  label,
  sub,
  value,
  badge,
}: {
  label: string
  sub?: string
  value?: string
  badge?: StatusBadge
}) => (
  <div
    style={{
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'space-between',
      gap: 12,
      padding: '11px 16px',
      borderTop: '1px solid var(--mtp-border)',
    }}
  >
    <div style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
      <span
        style={{
          fontSize: 13,
          fontWeight: 500,
          color: 'var(--mtp-text)',
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
        }}
      >
        {label}
      </span>
      {sub && <span style={{ fontSize: 12, color: 'var(--mtp-text-2)' }}>{sub}</span>}
    </div>
    <div style={{ display: 'flex', alignItems: 'center', gap: 10, flex: '0 0 auto' }}>
      {value && (
        <Mono style={{ fontSize: 13, fontWeight: 500, color: 'var(--mtp-text)' }}>{value}</Mono>
      )}
      {badge && <Pill badge={badge} />}
    </div>
  </div>
)

/* --------------------------------------------------------------- plan widget */

const PlanEmbed = ({ overview }: { overview: CustomerPortalOverview }) => {
  const [expanded, setExpanded] = useState(false)
  const subs = overview.activeSubscriptions
  const primary = subs[0] as SubscriptionSummary | undefined

  if (!primary) {
    return (
      <Card>
        <CenterState
          title="No active plan"
          hint="There's no active subscription on this account yet."
          action={
            <PButton size="sm" onClick={() => requestNavigate('subscriptions')}>
              Manage billing
            </PButton>
          }
        />
      </Card>
    )
  }

  return (
    <Card pad={20} style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'flex-start',
          justifyContent: 'space-between',
          gap: 12,
        }}
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8, minWidth: 0 }}>
          <Eyebrow>Current plan</Eyebrow>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <span style={{ fontSize: 18, fontWeight: 600, letterSpacing: '-0.02em' }}>
              {primary.planName}
            </span>
            <Pill badge={subStatusBadge(primary.status)} dot />
          </div>
        </div>
        <PButton size="sm" onClick={() => requestNavigate('subscriptions')}>
          Manage
        </PButton>
      </div>

      <div style={{ display: 'flex', alignItems: 'baseline', gap: 8 }}>
        <Mono style={{ fontSize: 26, fontWeight: 600, letterSpacing: '-0.03em' }}>
          {money(primary.mrrCents, primary.currency)}
        </Mono>
        <span style={{ fontSize: 13, color: 'var(--mtp-text-2)' }}>/ mo</span>
      </div>

      {expanded ? (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {primary.nextBillingDate && (
            <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 12.5 }}>
              <span style={{ color: 'var(--mtp-text-2)' }}>Next charge</span>
              <span style={{ color: 'var(--mtp-text)' }}>{date(primary.nextBillingDate)}</span>
            </div>
          )}
          {subs.length > 1 && (
            <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 12.5 }}>
              <span style={{ color: 'var(--mtp-text-2)' }}>Other subscriptions</span>
              <span style={{ color: 'var(--mtp-text)' }}>{subs.length - 1}</span>
            </div>
          )}
        </div>
      ) : (
        primary.nextBillingDate && (
          <button
            type="button"
            onClick={() => setExpanded(true)}
            className="mtp-link"
            style={{
              alignSelf: 'flex-start',
              fontSize: 12.5,
              fontWeight: 500,
              color: 'var(--mtp-accent-ink)',
              background: 'none',
              border: 'none',
              cursor: 'pointer',
              fontFamily: 'inherit',
              padding: 0,
            }}
          >
            Details
          </button>
        )
      )}
    </Card>
  )
}

/* ------------------------------------------------------ subscriptions widget */

const SubscriptionsEmbed = ({ overview }: { overview: CustomerPortalOverview }) => {
  const subs = overview.activeSubscriptions

  if (subs.length === 0) {
    return (
      <Card>
        <CenterState
          title="No subscriptions"
          hint="Your active subscriptions will appear here."
          action={
            <PButton size="sm" onClick={() => requestNavigate('subscriptions')}>
              View billing
            </PButton>
          }
        />
      </Card>
    )
  }

  return (
    <PanelCard
      title="Subscriptions"
      action={
        <button
          type="button"
          onClick={() => requestNavigate('subscriptions')}
          className="mtp-link"
          style={{
            fontSize: 12,
            fontWeight: 500,
            color: 'var(--mtp-accent-ink)',
            background: 'none',
            border: 'none',
            cursor: 'pointer',
            fontFamily: 'inherit',
            padding: 0,
          }}
        >
          View details
        </button>
      }
    >
      {subs.map(sub => (
        <Row
          key={sub.id}
          label={sub.planName}
          sub={
            sub.pendingCancellationDate
              ? `cancels ${date(sub.pendingCancellationDate)}`
              : sub.nextBillingDate
                ? `renews ${date(sub.nextBillingDate)}`
                : undefined
          }
          value={`${money(sub.mrrCents, sub.currency)} / mo`}
          badge={subStatusBadge(sub.status)}
        />
      ))}
    </PanelCard>
  )
}

/* -------------------------------------------------- single subscription widget */

const SubscriptionEmbed = () => {
  const subscriptionId = useMemo(getSubscriptionId, [])
  const query = useQuery(
    getSubscriptionDetails,
    { subscriptionId: subscriptionId ?? '' },
    { enabled: !!subscriptionId }
  )

  if (!subscriptionId) {
    return (
      <Card>
        <CenterState
          title="No subscription selected"
          hint="Pass a subscription id to this embed (the `subscription` option)."
        />
      </Card>
    )
  }
  if (query.isLoading) return <LoadingShell />
  if (query.error) return <ErrorShell />

  const sub = query.data?.subscription
  if (!sub) {
    return (
      <Card>
        <CenterState title="Subscription not found" hint="It may have been removed." />
      </Card>
    )
  }

  const headline = feeLabel(sub.headlineFee, sub.currency)
  const pending = sub.pendingEvent
  const isCancelScheduled = pending?.eventType === PendingEventType.CANCEL

  return (
    <Card pad={20} style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'flex-start',
          justifyContent: 'space-between',
          gap: 12,
        }}
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8, minWidth: 0 }}>
          <Eyebrow>Subscription</Eyebrow>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <span style={{ fontSize: 18, fontWeight: 600, letterSpacing: '-0.02em' }}>
              {sub.planName}
            </span>
            <Pill badge={subStatusBadge(sub.status)} dot />
          </div>
        </div>
        {sub.canChangePlan && (
          <PButton size="sm" onClick={() => requestNavigate('subscriptions')}>
            Manage
          </PButton>
        )}
      </div>

      <Mono style={{ fontSize: 22, fontWeight: 600, letterSpacing: '-0.03em' }}>{headline}</Mono>

      {isCancelScheduled && pending?.scheduledDate ? (
        <span style={{ fontSize: 12.5, color: 'var(--mtp-danger)' }}>
          Cancels {date(pending.scheduledDate)}
        </span>
      ) : sub.currentPeriodEnd ? (
        <span style={{ fontSize: 12.5, color: 'var(--mtp-text-2)' }}>
          Renews {date(sub.currentPeriodEnd)}
        </span>
      ) : null}
    </Card>
  )
}

/* -------------------------------------------------------------- usage widget */

const UsageEmbed = ({ overview }: { overview: CustomerPortalOverview }) => (
  <UsageList subs={overview.activeSubscriptions} />
)

/* ----------------------------------------------------------- invoices widget */

const invoiceBadge = (invoice: InvoiceSummary): StatusBadge => {
  if (invoice.status === InvoiceStatus.DRAFT) return { label: 'Draft', tone: 'neutral' }
  switch (invoice.paymentStatus) {
    case InvoicePaymentStatus.PAID:
      return { label: 'Paid', tone: 'neutral' }
    case InvoicePaymentStatus.PARTIALLY_PAID:
      return { label: 'Partially paid', tone: 'warn' }
    case InvoicePaymentStatus.ERRORED:
      return { label: 'Errored', tone: 'danger' }
    case InvoicePaymentStatus.PROCESSING:
      return { label: 'Processing', tone: 'warn' }
    case InvoicePaymentStatus.UNPAID:
    default:
      return { label: 'Unpaid', tone: 'warn' }
  }
}

/** Rows per page for the invoices widget; from `?count=`, clamped to 1..20. */
const getInvoiceCount = (): number => {
  if (typeof window === 'undefined') return 5
  const raw = new URLSearchParams(window.location.search).get('count')
  const n = raw ? parseInt(raw, 10) : NaN
  return Number.isFinite(n) ? Math.min(20, Math.max(1, n)) : 5
}

const InvoicesEmbed = () => {
  const perPage = useMemo(getInvoiceCount, [])
  const [page, setPage] = useState(1)

  // The backend pagination is 0-based; our visible page starts at 1.
  const query = useInvoices(page - 1, perPage)

  if (query.isLoading) return <LoadingShell />
  if (query.error) return <ErrorShell />

  const invoices = query.data?.invoices ?? []
  const totalPages = query.data?.paginationMeta?.totalPages ?? 0
  const hasNext = totalPages > 0 ? page < totalPages : invoices.length === perPage
  const hasPrev = page > 1

  if (invoices.length === 0 && page === 1) {
    return (
      <Card>
        <CenterState title="No invoices yet" hint="Invoices will show up here as they're issued." />
      </Card>
    )
  }

  return (
    <PanelCard
      title="Recent invoices"
      action={
        <button
          type="button"
          onClick={() => requestNavigate('invoices')}
          className="mtp-link"
          style={{
            fontSize: 12,
            fontWeight: 500,
            color: 'var(--mtp-accent-ink)',
            background: 'none',
            border: 'none',
            cursor: 'pointer',
            fontFamily: 'inherit',
            padding: 0,
          }}
        >
          View all
        </button>
      }
    >
      {invoices.map(inv => (
        <Row
          key={inv.id}
          label={inv.invoiceNumber || 'Invoice'}
          sub={date(inv.invoiceDate)}
          value={money(inv.totalCents, inv.currency)}
          badge={invoiceBadge(inv)}
        />
      ))}

      {(hasPrev || hasNext) && (
        <div
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            gap: 12,
            padding: '11px 16px',
            borderTop: '1px solid var(--mtp-border)',
          }}
        >
          <span style={{ fontSize: 12, color: 'var(--mtp-text-3)' }}>
            Page {page}
            {totalPages > 0 ? ` of ${totalPages}` : ''}
          </span>
          <div style={{ display: 'flex', gap: 8 }}>
            <PButton
              variant="secondary"
              size="sm"
              disabled={!hasPrev}
              onClick={() => setPage(p => Math.max(1, p - 1))}
            >
              Previous
            </PButton>
            <PButton
              variant="secondary"
              size="sm"
              disabled={!hasNext}
              onClick={() => setPage(p => p + 1)}
            >
              Next
            </PButton>
          </div>
        </div>
      )}
    </PanelCard>
  )
}

/* --------------------------------------------------- payment-methods widget */

const PaymentMethodsEmbed = ({
  overview,
  refetch,
}: {
  overview: CustomerPortalOverview
  refetch: () => void
}) => {
  const methods = overview.paymentMethods
  const defaultId = overview.customer?.currentPaymentMethodId
  const canAdd = !!(overview.cardConnectionId || overview.directDebitConnectionId)

  const [addOpen, setAddOpen] = useState(false)
  const setDefaultMut = useMutation(setDefaultPaymentMethod)
  const [defaultingId, setDefaultingId] = useState<string | null>(null)

  const handleSetDefault = async (id: string) => {
    setDefaultingId(id)
    try {
      await setDefaultMut.mutateAsync({ paymentMethodId: id })
      refetch()
    } catch (e) {
      toast.error(e instanceof Error ? e.message : 'Unable to set default')
    } finally {
      setDefaultingId(null)
    }
  }

  const addButton = canAdd && (
    <PButton size="sm" onClick={() => setAddOpen(true)}>
      Add
    </PButton>
  )

  const dialog = (
    <AddPaymentMethodDialog
      open={addOpen}
      onOpenChange={setAddOpen}
      cardConnectionId={overview.cardConnectionId}
      directDebitConnectionId={overview.directDebitConnectionId}
      onSuccess={() => refetch()}
    />
  )

  if (methods.length === 0) {
    return (
      <Card>
        <CenterState
          title="No payment methods"
          hint="Add a card or bank account to manage billing."
          action={canAdd ? addButton : undefined}
        />
        {dialog}
      </Card>
    )
  }

  return (
    <Card pad={0} style={{ display: 'flex', flexDirection: 'column' }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 12,
          padding: '16px 16px 14px',
        }}
      >
        <Eyebrow>Payment methods</Eyebrow>
        {addButton}
      </div>

      {methods.map(pm => {
        const isDefault = pm.id === defaultId
        return (
          <div
            key={pm.id}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 12,
              padding: '12px 16px',
              borderTop: '1px solid var(--mtp-border)',
            }}
          >
            <BrandChip pm={pm} />
            <div style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
              <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--mtp-text)' }}>
                {pmLabel(pm)}
              </span>
              <span style={{ fontSize: 12, color: 'var(--mtp-text-3)' }}>{pmSubLabel(pm)}</span>
            </div>
            <span style={{ marginLeft: 'auto', flex: '0 0 auto' }}>
              {isDefault ? (
                <Pill badge={{ label: 'Default', tone: 'ok' }} dot />
              ) : (
                <PButton
                  size="sm"
                  variant="ghost"
                  loading={defaultingId === pm.id}
                  onClick={() => handleSetDefault(pm.id)}
                >
                  Set default
                </PButton>
              )}
            </span>
          </div>
        )
      })}

      <div style={{ padding: '12px 16px', borderTop: '1px solid var(--mtp-border)' }}>
        <PButton size="sm" variant="ghost" onClick={() => requestNavigate('settings')}>
          Manage in portal
        </PButton>
      </div>

      {dialog}
    </Card>
  )
}

/* ------------------------------------------------------------ compact loader */

/**
 * Shared shell for the compact (non-portal) widgets: loads the overview and
 * renders the supplied view, with loading/error states. The `refetch` passed to
 * the render callback re-pulls the overview after a mutation (e.g. set-default).
 */
const CompactEmbed = ({
  render,
}: {
  render: (overview: CustomerPortalOverview, refetch: () => void) => JSX.Element
}) => {
  const { overview, isLoading, error, refetch } = usePortalOverview()
  if (isLoading) return <LoadingShell />
  if (error || !overview) return <ErrorShell />
  return render(overview, () => void refetch())
}

/* --------------------------------------------------------------- powered by */

/** Compact "Powered by Meteroid" attribution shown under each widget. */
const PoweredBy = () => (
  <div
    style={{
      marginTop: 10,
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      gap: 6,
    }}
  >
    <span style={{ fontSize: 11, color: 'var(--mtp-text-3)' }}>Powered by</span>
    <a
      href="https://meteroid.com/?utm_source=portal-embed"
      target="_blank"
      rel="noopener noreferrer"
      style={{
        fontSize: 11.5,
        fontWeight: 600,
        color: 'var(--mtp-text-2)',
        textDecoration: 'none',
      }}
    >
      Meteroid
    </a>
  </div>
)

/* --------------------------------------------------------------- host entry */

const WIDGET_WRAP: CSSProperties = { padding: 12 }

export const EmbedHost = ({ view }: { view: EmbedView }) => {
  const ref = useAutoHeight<HTMLDivElement>()
  const { branding } = usePortalConfig()

  // The full portal renders its own footer; only the compact widgets need one.
  if (view === 'portal') {
    return (
      <div ref={ref}>
        <PortalApp />
      </div>
    )
  }

  return (
    <div ref={ref} style={WIDGET_WRAP}>
      {view === 'plan' && <CompactEmbed render={o => <PlanEmbed overview={o} />} />}
      {view === 'subscriptions' && (
        <CompactEmbed render={o => <SubscriptionsEmbed overview={o} />} />
      )}
      {view === 'subscription' && <SubscriptionEmbed />}
      {view === 'usage' && <CompactEmbed render={o => <UsageEmbed overview={o} />} />}
      {view === 'invoices' && <InvoicesEmbed />}
      {view === 'payment-methods' && (
        <CompactEmbed
          render={(o, refetch) => <PaymentMethodsEmbed overview={o} refetch={refetch} />}
        />
      )}
      {branding.showPoweredBy !== false && <PoweredBy />}
    </div>
  )
}
