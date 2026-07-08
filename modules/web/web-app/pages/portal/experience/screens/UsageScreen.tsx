import { ChevronDown } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { getUpcomingInvoice } from '@/rpc/portal/subscription/v1/subscription-PortalSubscriptionService_connectquery'

import { UsageChart } from '../UsageChart'
import { usePortalNav } from '../context'
import { date, money } from '../format'
import { CenterState, Mono, PanelCard, Spinner } from '../primitives'

import type { LineItem, SubLineItem } from '@/rpc/api/invoices/v1/models_pb'
import type { SubscriptionSummary } from '@/rpc/portal/customer/v1/models_pb'

/** Format a decimal-string quantity compactly, dropping trailing zeros. */
const fmtQty = (raw?: string): string | undefined => {
  if (!raw) return undefined
  const n = Number(raw)
  if (Number.isNaN(n)) return raw
  return new Intl.NumberFormat('en-US', { maximumFractionDigits: 4 }).format(n)
}

const SubLineRow = ({ sub, currency }: { sub: SubLineItem; currency: string }) => {
  const qty = fmtQty(sub.quantity)
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '7px 0' }}>
      <span
        style={{
          fontSize: 12.5,
          color: 'var(--mtp-text-2)',
          flex: 1,
          minWidth: 0,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {sub.name || '—'}
      </span>
      {qty && (
        <Mono style={{ fontSize: 12, color: 'var(--mtp-text-3)', whiteSpace: 'nowrap' }}>
          {qty}
        </Mono>
      )}
      <Mono style={{ fontSize: 12.5, color: 'var(--mtp-text-2)', whiteSpace: 'nowrap', minWidth: 72, textAlign: 'right' }}>
        {money(sub.total, currency)}
      </Mono>
    </div>
  )
}

/** A single metered line item with an (expandable) daily-usage chart. */
const UsageItemRow = ({
  item,
  currency,
  subscriptionId,
  isFirst,
  defaultExpanded,
}: {
  item: LineItem
  currency: string
  subscriptionId: string
  isFirst: boolean
  defaultExpanded?: boolean
}) => {
  const qty = fmtQty(item.quantity)
  const [showUsage, setShowUsage] = useState(defaultExpanded ?? false)
  return (
    <div
      style={{
        padding: '14px 20px',
        borderTop: isFirst ? 'none' : '1px solid var(--mtp-border)',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
        <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 2 }}>
          <span style={{ fontSize: 13.5, fontWeight: 500, color: 'var(--mtp-text)' }}>
            {item.name || '—'}
          </span>
          {item.description && (
            <span style={{ fontSize: 12, color: 'var(--mtp-text-3)' }}>{item.description}</span>
          )}
        </div>
        {qty && (
          <Mono style={{ fontSize: 12.5, color: 'var(--mtp-text-2)', whiteSpace: 'nowrap' }}>
            {qty}
          </Mono>
        )}
        <Mono
          style={{
            fontSize: 13.5,
            fontWeight: 500,
            color: 'var(--mtp-text)',
            whiteSpace: 'nowrap',
            minWidth: 88,
            textAlign: 'right',
          }}
        >
          {money(item.subtotal, currency)}
        </Mono>
      </div>

      {item.subLineItems.length > 0 && (
        <div
          style={{
            marginTop: 8,
            marginLeft: 12,
            paddingLeft: 12,
            borderLeft: '2px solid var(--mtp-border)',
          }}
        >
          {item.subLineItems.map(sub => (
            <SubLineRow key={sub.id} sub={sub} currency={currency} />
          ))}
        </div>
      )}

      <div style={{ marginTop: item.subLineItems.length > 0 ? 12 : 10 }}>
        <button
          type="button"
          onClick={() => setShowUsage(s => !s)}
          className="mtp-link"
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 5,
            fontSize: 12,
            fontWeight: 500,
            color: 'var(--mtp-text-2)',
            background: 'none',
            border: 'none',
            padding: 0,
            cursor: 'pointer',
          }}
        >
          <ChevronDown
            size={13}
            style={{ transition: 'transform 0.15s', transform: showUsage ? 'none' : 'rotate(-90deg)' }}
          />
          {showUsage ? 'Hide daily usage' : 'Show daily usage'}
        </button>
        {showUsage && (
          <div style={{ marginTop: 10 }}>
            <UsageChart
              subscriptionId={subscriptionId}
              metricId={item.metricId!}
              startDate={item.startDate || undefined}
              endDate={item.endDate || undefined}
              groupByDimensions={
                Object.keys(item.groupByDimensions).length > 0 ? item.groupByDimensions : undefined
              }
            />
          </div>
        )}
      </div>
    </div>
  )
}

/**
 * Usage panel for one subscription: its metered line items only, each with a
 * daily-usage chart. Renders nothing when the subscription has no metered usage
 * this cycle, so the Usage tab stays focused on usage-based charges.
 */
const SubscriptionUsage = ({
  sub,
  expandCharts,
  onResolve,
}: {
  sub: SubscriptionSummary
  expandCharts?: boolean
  onResolve: (id: string, hasUsage: boolean) => void
}) => {
  const query = useQuery(getUpcomingInvoice, { subscriptionId: sub.id }, { enabled: !!sub.id })
  const invoice = query.data?.invoice

  // Usage-based charges are the metered line items (they carry a metric_id).
  const usageItems = invoice?.lineItems.filter(li => li.metricId) ?? []
  const settled = !query.isLoading

  useEffect(() => {
    if (settled) onResolve(sub.id, usageItems.length > 0)
  }, [settled, usageItems.length, sub.id, onResolve])

  if (query.isLoading) {
    return (
      <PanelCard title={sub.planName}>
        <div style={{ display: 'flex', justifyContent: 'center', padding: '28px 0' }}>
          <Spinner size={16} />
        </div>
      </PanelCard>
    )
  }

  if (usageItems.length === 0) return null

  const currency = invoice!.currency
  const meteredTotal = usageItems.reduce((acc, li) => acc + li.subtotal, 0n)

  return (
    <PanelCard
      title={sub.planName}
      action={
        <span style={{ fontSize: 12, color: 'var(--mtp-text-3)' }}>
          {date(invoice!.periodStart)} – {date(invoice!.periodEnd)} · {money(meteredTotal, currency)}
        </span>
      }
    >
      {usageItems.map((item, i) => (
        <UsageItemRow
          key={item.id}
          item={item}
          currency={currency}
          subscriptionId={sub.id}
          isFirst={i === 0}
          defaultExpanded={expandCharts}
        />
      ))}
    </PanelCard>
  )
}

/**
 * Metered-usage panels for a set of subscriptions, with empty states. Free of
 * portal-nav context so both the full Usage screen and the `usage` embed widget
 * can render it; the caller supplies the subscriptions.
 */
export const UsageList = ({ subs }: { subs: SubscriptionSummary[] }) => {
  // Each subscription panel reports whether it has metered usage once its
  // upcoming invoice settles, so we can show a single empty state when none do.
  const [resolved, setResolved] = useState<Record<string, boolean>>({})
  const onResolve = useCallback((id: string, hasUsage: boolean) => {
    setResolved(prev => (prev[id] === hasUsage ? prev : { ...prev, [id]: hasUsage }))
  }, [])

  if (subs.length === 0) {
    return (
      <CenterState
        title="No usage to show"
        hint="Usage will appear here once you have an active subscription with metered charges."
      />
    )
  }

  const allSettled = subs.every(s => s.id in resolved)
  const anyUsage = subs.some(s => resolved[s.id])

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 22 }}>
      {subs.map(sub => (
        <SubscriptionUsage
          key={sub.id}
          sub={sub}
          expandCharts={subs.length === 1}
          onResolve={onResolve}
        />
      ))}

      {allSettled && !anyUsage && (
        <CenterState
          title="No metered usage yet"
          hint="None of your subscriptions have accrued usage-based charges this billing period."
        />
      )}
    </div>
  )
}

export const UsageScreen = () => {
  const { overview } = usePortalNav()

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 22 }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        <h1 style={{ fontSize: 22, fontWeight: 600, letterSpacing: '-0.02em', margin: 0 }}>Usage</h1>
        <p style={{ fontSize: 13, color: 'var(--mtp-text-2)', margin: 0 }}>
          Metered usage across your active subscriptions this billing period.
        </p>
      </div>

      <UsageList subs={overview.activeSubscriptions} />
    </div>
  )
}
