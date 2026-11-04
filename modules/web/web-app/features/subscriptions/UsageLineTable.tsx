import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@md/ui'
import Decimal from 'decimal.js'
import { ChevronDown } from 'lucide-react'
import { useId, useState } from 'react'

import { formatPerUnit, formatRawQuantity, unitPlural } from '@/features/billing/metrics/units'
import { formatWalletAmount } from '@/features/credit/format'
import { UsageBarChart } from '@/features/subscriptions/UsageBarChart'
import { creditUnitNames } from '@/features/subscriptions/subscriptionPrice'
import { formatCurrency, formatCurrencyNoRounding } from '@/lib/utils/numbers'

import type { MetricUnit } from '@/features/billing/metrics/units'
import type { CreditUnitNames } from '@/features/subscriptions/subscriptionPrice'
import type { LineItem, SubLineItem } from '@/rpc/api/invoices/v1/models_pb'

// More rows than this collapse behind a toggle so the invoice stays scannable.
export const INLINE_USAGE_ROWS = 6

const capitalize = (value: string) =>
  value ? value[0].toUpperCase() + value.slice(1).replace(/_/g, ' ') : value

const sublineName = (sub: SubLineItem) => {
  if (sub.sublineAttributes.case !== 'matrix') return sub.name
  const m = sub.sublineAttributes.value
  return m.dimension2Value ? `${m.dimension1Value} / ${m.dimension2Value}` : m.dimension1Value
}

/** The column naming the rows: the matrix dimension ("Model"), else the line's breakdown. */
export const breakdownHeader = (subs: SubLineItem[]) => {
  for (const sub of subs) {
    if (sub.sublineAttributes.case === 'matrix') {
      const m = sub.sublineAttributes.value
      return m.dimension2Key
        ? `${capitalize(m.dimension1Key)} / ${capitalize(m.dimension2Key)}`
        : capitalize(m.dimension1Key)
    }
  }
  return 'Item'
}

/** Billed quantity in priced units: a package subline counts packages, its usage is `rawUsage`. */
const billedQuantity = (sub: SubLineItem) =>
  sub.sublineAttributes.case === 'package' ? sub.sublineAttributes.value.rawUsage : sub.quantity

const formatQuantity = (quantity: string | undefined, unit?: MetricUnit) => {
  if (unit) return formatRawQuantity(quantity, unit)
  if (!quantity) return '—'
  const n = Number(quantity)
  return Number.isFinite(n) ? n.toLocaleString(undefined, { maximumFractionDigits: 6 }) : quantity
}

const bareNumber = (value: string) => {
  try {
    return new Intl.NumberFormat(undefined, { maximumFractionDigits: 12 }).format(
      new Decimal(value).toNumber()
    )
  } catch {
    return value
  }
}

/** A usage line's breakdown inside the next invoice; credit-unit amounts are bare credits. */
export const UsageLineTable = ({
  line,
  currency,
  subscriptionId,
  unit,
  creditUnits,
}: {
  line: LineItem
  currency: string
  subscriptionId: string
  unit?: MetricUnit
  creditUnits: Record<string, CreditUnitNames>
}) => {
  const [showChart, setShowChart] = useState(false)
  const credit = line.creditUnit
  const names = credit ? creditUnitNames(credit, creditUnits) : undefined
  const subs = credit ? credit.subLineItems : line.subLineItems
  const collapsible = subs.length > INLINE_USAGE_ROWS
  const [expanded, setExpanded] = useState(!collapsible)
  const rowsId = useId()
  const chartId = useId()

  const amount = (minor: bigint) =>
    credit && names
      ? formatWalletAmount(minor, { currency: credit.code, creditUnit: names }, { bare: true })
      : formatCurrency(minor, currency)
  const price = (value: string | undefined) => {
    if (!value) return '—'
    return credit ? bareNumber(value) : formatCurrencyNoRounding(value, currency)
  }

  const rows =
    subs.length > 0
      ? subs.map(sub => ({
          key: sub.id || sublineName(sub),
          name: sublineName(sub),
          quantity: billedQuantity(sub),
          price:
            sub.sublineAttributes.case === 'package'
              ? `${price(sub.unitPrice)} per package`
              : price(sub.unitPrice),
          total: sub.total,
        }))
      : [
          {
            key: line.id || line.name,
            name: line.name,
            quantity: line.quantity,
            price: price(credit ? credit.unitPrice : line.unitPrice),
            total: credit ? credit.consumed : line.subtotal,
          },
        ]
  const num = 'text-right tabular-nums'
  const header = subs.length > 0 ? breakdownHeader(subs) : 'Item'
  const per = unit ? formatPerUnit(unit) : 'per unit'

  return (
    <div className="flex flex-col gap-1.5 pl-3">
      {collapsible && (
        <button
          type="button"
          aria-expanded={expanded}
          aria-controls={rowsId}
          className="flex w-fit items-center gap-1 text-xs text-brand hover:underline"
          onClick={() => setExpanded(!expanded)}
        >
          <ChevronDown className={`h-3 w-3 transition-transform ${expanded ? '' : '-rotate-90'}`} />
          {rows.length} {header.toLowerCase()}s
        </button>
      )}
      {expanded && (
        <Table id={rowsId}>
          <TableHeader className="border-t-0 bg-transparent">
            <TableRow className="hover:bg-transparent">
              <TableHead className="h-8 pl-0">{header}</TableHead>
              <TableHead className="h-8 text-right">
                {unit ? capitalize(unitPlural(unit)) : 'Quantity'}
              </TableHead>
              <TableHead className="h-8 text-right">
                {credit ? `Credits ${per}` : `Price ${per}`}
              </TableHead>
              <TableHead className="h-8 pr-0 text-right">{credit ? 'Credits' : 'Amount'}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map(row => (
              <TableRow key={row.key} className="hover:bg-transparent">
                <TableCell className="py-1.5 pl-0 text-foreground">{row.name}</TableCell>
                <TableCell className={`py-1.5 ${num}`}>
                  {formatQuantity(row.quantity, unit)}
                </TableCell>
                <TableCell className={`py-1.5 ${num} text-muted-foreground`}>{row.price}</TableCell>
                <TableCell className={`py-1.5 ${num} pr-0`}>{amount(row.total)}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
      {line.metricId && (
        <div>
          <button
            type="button"
            aria-expanded={showChart}
            aria-controls={chartId}
            className="flex items-center gap-1 text-xs text-brand hover:underline"
            onClick={() => setShowChart(!showChart)}
          >
            <ChevronDown
              className={`h-3 w-3 transition-transform ${showChart ? '' : '-rotate-90'}`}
            />
            {showChart ? 'Hide daily usage' : 'Show daily usage'}
          </button>
          {showChart && (
            <div id={chartId} className="mt-2">
              <UsageBarChart
                subscriptionId={subscriptionId}
                metricId={line.metricId}
                startDate={line.startDate || undefined}
                endDate={line.endDate || undefined}
                groupByDimensions={
                  Object.keys(line.groupByDimensions).length > 0
                    ? line.groupByDimensions
                    : undefined
                }
              />
            </div>
          )}
        </div>
      )}
    </div>
  )
}
