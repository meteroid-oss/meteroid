import { Skeleton, cn } from '@md/ui'
import Decimal from 'decimal.js'

import { formatMetricQuantity, rawQuantity } from '@/features/billing/metrics/units'
import { formatWalletAmount, walletTitle } from '@/features/credit/format'
import { formatApprox } from '@/features/subscriptions/PeriodUsageBar'
import { SubscriptionSectionCard } from '@/features/subscriptions/SubscriptionSectionCard'
import { UsageLineTable } from '@/features/subscriptions/UsageLineTable'
import {
  issueDay,
  nextInvoiceTitle,
  packSplit,
  unitShortName,
} from '@/features/subscriptions/subscriptionOverview'
import { creditUnitNames, toMinor } from '@/features/subscriptions/subscriptionPrice'
import { isUsageLine } from '@/features/subscriptions/subscriptionWallets'
import { formatShortDay } from '@/features/subscriptions/utils/dates'
import { useQuery } from '@/lib/connectrpc'
import { currencyPrecision } from '@/lib/data/currency-precision'
import { formatDateOnly } from '@/lib/utils/date'
import { formatCurrency, rateToPercent } from '@/lib/utils/numbers'
import { LineItemKind } from '@/rpc/api/invoices/v1/models_pb'
import { SubscriptionStatus } from '@/rpc/api/subscriptions/v1/models_pb'
import {
  getSubscriptionMinimumProgress,
  getUpcomingInvoice,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type { MetricUnit } from '@/features/billing/metrics/units'
import type { CreditUnitNames } from '@/features/subscriptions/subscriptionPrice'
import type { LineItem } from '@/rpc/api/invoices/v1/models_pb'
import type {
  Subscription,
  SubscriptionAddOn,
  SubscriptionComponent,
  SubscriptionFee,
} from '@/rpc/api/subscriptions/v1/models_pb'
import type { Wallet } from '@/rpc/api/wallets/v1/models_pb'
import type { ReactNode } from 'react'

const lineRange = (lines: LineItem[]) => {
  const starts = lines.map(l => l.startDate).filter(Boolean)
  const ends = lines.map(l => l.endDate).filter(Boolean)
  if (starts.length === 0 || ends.length === 0) return undefined
  const start = starts.reduce((a, b) => (new Date(a) <= new Date(b) ? a : b))
  const end = ends.reduce((a, b) => (new Date(a) >= new Date(b) ? a : b))
  return `${formatShortDay(start)} – ${formatShortDay(end)}`
}

// Upcoming-invoice line ids are regenerated on every computation: keying rows by them would
// remount each row on refetch and reset its toggles.
const stableLineKeys = (lines: LineItem[]) => {
  const seen = new Map<string, number>()
  return new Map(
    lines.map(line => {
      const dims = Object.entries(line.groupByDimensions)
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([k, v]) => `${k}=${v}`)
        .join(',')
      const base = [
        line.kind,
        line.priceComponentId ?? '',
        line.metricId ?? '',
        dims,
        line.name,
        line.startDate,
        line.endDate,
      ].join('|')
      const n = seen.get(base) ?? 0
      seen.set(base, n + 1)
      return [line, n === 0 ? base : `${base}#${n}`] as const
    })
  )
}

type CreditPackFee = { name: string; priceComponentId?: string; fee: SubscriptionFee }

/** The credit pack a CREDIT line bills: by plan component, else by name, else the only pack. */
const packOf = (line: LineItem, packs: CreditPackFee[]): CreditPackFee | undefined =>
  (line.priceComponentId
    ? packs.find(p => p.priceComponentId === line.priceComponentId)
    : undefined) ??
  packs.find(p => p.name === line.name) ??
  (packs.length === 1 ? packs[0] : undefined)

export const SubscriptionNextInvoice = ({
  subscription,
  components,
  addOns,
  paymentMethod,
  wallets,
  hasMinimum,
  hasMeteredFees,
  hideWhenFree = false,
  units,
  creditUnits,
}: {
  subscription: Subscription
  components: SubscriptionComponent[]
  addOns: SubscriptionAddOn[]
  paymentMethod?: string
  /** The customer's wallets paying this subscription. */
  wallets: Wallet[]
  hasMinimum: boolean
  hasMeteredFees: boolean
  /** A Free plan with nothing to bill shows no invoice; the header says "No charge". */
  hideWhenFree?: boolean
  units: Record<string, MetricUnit | undefined>
  creditUnits: Record<string, CreditUnitNames>
}) => {
  const invoiceQuery = useQuery(getUpcomingInvoice, { subscriptionId: subscription.id })
  const progressQuery = useQuery(
    getSubscriptionMinimumProgress,
    { subscriptionId: subscription.id },
    { enabled: hasMinimum, staleTime: 60_000 }
  )
  const currency = subscription.currency
  const chargeAutomatically = subscription.chargeAutomatically
  const trialing = subscription.status === SubscriptionStatus.TRIALING
  const loadingTitle = trialing ? 'First invoice' : 'Next invoice'

  if (invoiceQuery.isLoading) {
    return (
      <SubscriptionSectionCard title={loadingTitle} collapsible defaultOpen={false}>
        <div className="flex flex-col gap-3 p-4">
          <Skeleton height={14} width={240} />
          <Skeleton height={120} />
        </div>
      </SubscriptionSectionCard>
    )
  }
  if (invoiceQuery.isError) {
    return (
      <SubscriptionSectionCard title={loadingTitle}>
        <p className="p-4 text-sm text-muted-foreground">The next invoice could not be loaded.</p>
      </SubscriptionSectionCard>
    )
  }
  const invoice = invoiceQuery.data?.invoice
  if (!invoice || invoice.notInvoiced) return null
  if (hideWhenFree && invoice.amountDue === 0n && invoice.lineItems.length === 0) return null

  const { title, after } = nextInvoiceTitle(invoice, trialing)
  // Nothing is charged when credit or discounts cover the whole invoice.
  const charged = chargeAutomatically && invoice.amountDue > 0n
  const money = (minor: bigint) => formatCurrency(minor, currency)
  const packs: CreditPackFee[] = [
    ...components.map(c => ({ name: c.name, priceComponentId: c.priceComponentId, fee: c.fee })),
    ...addOns.map(a => ({ name: a.name, priceComponentId: undefined, fee: a.fee })),
  ].flatMap(p => {
    const fee = p.fee
    return fee && fee.fee.case === 'credit' ? [{ ...p, fee }] : []
  })

  const creditUsageParts = (line: LineItem): string[] => {
    const credit = line.creditUnit
    if (!credit) return []
    const names = creditUnitNames(credit, creditUnits)
    const denomination = { currency: credit.code, creditUnit: names }
    const credits = (minor: bigint) => formatWalletAmount(minor, denomination, { bare: true })
    const short = unitShortName(names)
    const beyond = credit.consumed - credit.covered
    return [
      `${credits(credit.consumed)} ${short} used`,
      beyond > 0n
        ? `${credits(beyond)} beyond credit · ${money(line.subtotal)}`
        : 'all covered by credit',
    ]
  }

  const progress = progressQuery.data?.progress
  const shortfallText = (line: LineItem) => {
    if (!progress) return line.description
    const outlook =
      progress.projectedShortfall > 0n
        ? `${formatApprox(progress.projectedShortfall, { currency })} projected by ${formatShortDay(progress.periodEnd)}`
        : 'minimum projected to be met'
    return `${money(progress.minimum)} minimum − ${money(progress.countedToDate)} counted so far · ${outlook}`
  }

  const packCreditText = (line: LineItem) => {
    const fee = packOf(line, packs)?.fee
    if (!fee || fee.fee.case !== 'credit') return undefined
    let credit: Decimal
    try {
      credit = new Decimal(fee.fee.value.creditTotal)
    } catch {
      return undefined
    }
    if (fee.creditUnit) {
      const names = creditUnitNames(fee.creditUnit, creditUnits)
      return `Adds ${formatWalletAmount(toMinor(credit, names.precision), {
        currency: fee.creditUnit.code,
        creditUnit: names,
      })}`
    }
    const face = line.creditGrantAmount ?? toMinor(credit, currencyPrecision(currency))
    const split = packSplit(face, line.taxableAmount)
    const splitText = split
      ? ` · ${money(split.paid)} paid + ${money(split.bonus)} bonus credit`
      : ''
    return `Adds ${money(face)} of credit${splitText}`
  }

  const usageText = (line: LineItem): ReactNode => {
    const unit = line.metricId ? units[line.metricId] : undefined
    const subs = line.creditUnit ? line.creditUnit.subLineItems : line.subLineItems
    let quantity: string | undefined = line.quantity
    if (subs.length > 0) {
      try {
        quantity = subs
          .reduce(
            (sum, s) =>
              sum.add(
                s.sublineAttributes.case === 'package'
                  ? s.sublineAttributes.value.rawUsage || '0'
                  : s.quantity || '0'
              ),
            new Decimal(0)
          )
          .toString()
      } catch {
        quantity = undefined
      }
    }
    const parts = [
      unit && quantity
        ? formatMetricQuantity(rawQuantity(quantity, unit) ?? quantity, unit)
        : undefined,
      ...(line.creditUnit ? creditUsageParts(line) : []),
    ].filter(Boolean)
    return parts.join(' · ') || undefined
  }

  const periodEnd = subscription.currentPeriodEnd
  const inAdvance = (line: LineItem) =>
    Boolean(periodEnd && line.startDate) &&
    new Date(line.startDate) >= new Date(periodEnd as string)
  const usageLines = invoice.lineItems.filter(isUsageLine)
  const shortfallLines = invoice.lineItems.filter(l => l.kind === LineItemKind.MINIMUM_SHORTFALL)
  const advanceLines = invoice.lineItems.filter(
    l => !isUsageLine(l) && l.kind !== LineItemKind.MINIMUM_SHORTFALL && inAdvance(l)
  )
  const otherLines = invoice.lineItems.filter(
    l => !isUsageLine(l) && l.kind !== LineItemKind.MINIMUM_SHORTFALL && !inAdvance(l)
  )

  const subText = (line: LineItem): ReactNode => {
    if (line.kind === LineItemKind.CREDIT) return packCreditText(line)
    if (line.kind === LineItemKind.MINIMUM_SHORTFALL) return shortfallText(line)
    if (isUsageLine(line)) return usageText(line)
    return line.description
  }

  const advanceRange = lineRange(advanceLines)
  const usageRange = lineRange(usageLines)
  const groups: { id: string; label?: string; lines: LineItem[] }[] = [
    {
      id: 'advance',
      label: `Billed in advance${advanceRange ? ` · ${advanceRange}` : ''}`,
      lines: advanceLines,
    },
    {
      id: 'usage',
      label: `Usage${usageRange ? ` · ${usageRange}` : ''} · so far`,
      lines: usageLines,
    },
    { id: 'shortfall', lines: shortfallLines },
    { id: 'other', lines: otherLines },
  ].filter(g => g.lines.length > 0)
  const noUsageYet = hasMeteredFees && usageLines.length === 0
  const lineKeys = stableLineKeys(invoice.lineItems)

  const subtitle = chargeAutomatically
    ? `Draft at today's usage · charged automatically${paymentMethod ? ` to ${paymentMethod}` : ''}`
    : `Draft at today's usage · ${
        invoice.dueDate
          ? `due ${formatDateOnly(invoice.dueDate)}`
          : invoice.netTerms > 0
            ? `net ${invoice.netTerms}`
            : 'due on receipt'
      }`
  // Paid credit applies after tax: the total separates the two only when both are present.
  const showTotal = invoice.appliedCredits > 0n && invoice.taxAmount > 0n
  // Money credit pays money lines: name the wallet only when exactly one could have.
  const moneyWallets = wallets.filter(w => !w.creditUnit)
  const withWallet = (label: string) =>
    moneyWallets.length === 1 ? `${label} · ${walletTitle(moneyWallets[0])}` : label
  // Shared credit is drawn by invoices billed before this one too: its amounts are estimates.
  const approx = invoice.creditEstimateShared ? '≈ ' : ''
  const moneyDeductions = invoice.creditDeductions.filter(
    d => d.currency.toUpperCase() === currency.toUpperCase()
  )
  // One row per kind of credit, unless the server's rows do not add up to the invoice's total.
  const creditRows = (promotional: boolean, total: bigint, fallback: string) => {
    const rows = moneyDeductions.filter(d => d.promotional === promotional)
    const sum = rows.reduce((acc, d) => acc + d.amount, 0n)
    if (rows.length > 0 && sum === total) {
      return rows.map(d => ({
        key: `${d.label}-${d.subscriptionId ?? ''}`,
        label: d.label,
        amount: d.amount,
      }))
    }
    return total > 0n ? [{ key: fallback, label: withWallet(fallback), amount: total }] : []
  }
  const billedFirst = invoice.creditEstimateShared
    ? invoice.billedFirst.map(
        b =>
          `${b.planName ?? 'An earlier invoice'} bills first, on ${formatShortDay(b.invoiceDate)}`
      )
    : []
  const taxLabel =
    invoice.taxBreakdown.length === 1 && invoice.taxBreakdown[0].name
      ? `Tax · ${invoice.taxBreakdown[0].name}${
          invoice.taxBreakdown[0].taxRate
            ? ` ${rateToPercent(invoice.taxBreakdown[0].taxRate)}%`
            : ''
        }`
      : 'Tax'

  return (
    <SubscriptionSectionCard
      title={`${title} · ${formatDateOnly(issueDay(invoice))}${after ? ` · ${after}` : ''}`}
      subtitle={subtitle}
      collapsible
      defaultOpen={false}
      aside={
        <div className="text-right">
          <div className="text-xl font-semibold tabular-nums text-foreground">
            {approx}
            {money(invoice.amountDue)}
          </div>
          <div className="text-xs text-muted-foreground">{charged ? 'to be charged' : 'due'}</div>
        </div>
      }
    >
      <div className="flex flex-col">
        {groups.map(group => (
          <div key={group.id} className="border-b border-border px-4 py-3">
            {group.label && (
              <div className="mb-2 text-xs font-medium text-muted-foreground">{group.label}</div>
            )}
            <div className="flex flex-col gap-2.5">
              {group.lines.map(line => (
                <div key={lineKeys.get(line)} className="flex flex-col gap-1.5">
                  <InvoiceLine line={line} sub={subText(line)} amount={money(line.subtotal)} />
                  {isUsageLine(line) && (
                    <UsageLineTable
                      line={line}
                      currency={currency}
                      subscriptionId={subscription.id}
                      unit={line.metricId ? units[line.metricId] : undefined}
                      creditUnits={creditUnits}
                    />
                  )}
                </div>
              ))}
            </div>
          </div>
        ))}
        {noUsageYet && (
          <div className="border-b border-border px-4 py-3 text-sm text-muted-foreground">
            No usage yet this period
          </div>
        )}
        <div className="flex flex-col gap-1.5 px-4 py-3 text-sm">
          <TotalRow label="Subtotal" value={money(invoice.subtotal)} />
          {invoice.couponLineItems.map((coupon, i) => (
            <TotalRow
              key={coupon.couponId || i}
              label={`Coupon: ${coupon.name}`}
              value={`−${money(coupon.total)}`}
            />
          ))}
          {/* The discount totals the coupons: shown alone only when no coupon row itemizes it. */}
          {invoice.discount > 0n && invoice.couponLineItems.length === 0 && (
            <TotalRow label="Discount" value={`−${money(invoice.discount)}`} />
          )}
          {creditRows(true, invoice.promotionalCredits, 'Free credit').map(row => (
            <TotalRow key={row.key} label={row.label} value={`${approx}−${money(row.amount)}`} />
          ))}
          {invoice.taxAmount > 0n && <TotalRow label={taxLabel} value={money(invoice.taxAmount)} />}
          {showTotal && <TotalRow label="Total" value={money(invoice.total)} />}
          {creditRows(false, invoice.appliedCredits, 'Paid with credit').map(row => (
            <TotalRow key={row.key} label={row.label} value={`${approx}−${money(row.amount)}`} />
          ))}
          <TotalRow
            strong
            label={charged ? `Charged on ${formatShortDay(issueDay(invoice))}` : 'Amount due'}
            value={`${approx}${money(invoice.amountDue)}`}
          />
          {billedFirst.length > 0 && (
            <p className="text-xs text-muted-foreground">
              Customer-wide credit · {billedFirst.join(' · ')}
            </p>
          )}
        </div>
      </div>
    </SubscriptionSectionCard>
  )
}

const InvoiceLine = ({ line, sub, amount }: { line: LineItem; sub: ReactNode; amount: string }) => (
  <div className="flex items-start justify-between gap-4 text-sm">
    <div className="min-w-0">
      <div className="flex items-center gap-1.5 text-foreground">
        <span>{line.name}</span>
        {line.isProrated && (
          <span className="rounded bg-muted px-1 text-[10px] text-muted-foreground">prorated</span>
        )}
      </div>
      {sub && <div className="text-xs text-muted-foreground">{sub}</div>}
    </div>
    <span className="shrink-0 font-medium tabular-nums text-foreground">{amount}</span>
  </div>
)

const TotalRow = ({
  label,
  value,
  strong = false,
}: {
  label: ReactNode
  value: string
  strong?: boolean
}) => (
  <div
    className={cn(
      'flex justify-between gap-4',
      strong
        ? 'mt-1 border-t border-border pt-2 font-semibold text-foreground'
        : 'text-muted-foreground'
    )}
  >
    <span>{label}</span>
    <span className="tabular-nums">{value}</span>
  </div>
)
