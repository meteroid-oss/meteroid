import { Skeleton } from '@md/ui'
import { ChevronDown, ChevronRight } from 'lucide-react'
import { useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { SubLineItem } from '@/rpc/api/invoices/v1/models_pb'
import { getUpcomingInvoice } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { UpcomingInvoice } from '@/rpc/api/subscriptions/v1/subscriptions_pb'
import { parseAndFormatDate } from '@/utils/date'
import { formatCurrency, formatCurrencyNoRounding } from '@/utils/numbers'

import { UsageBarChart } from './UsageBarChart'

interface UpcomingInvoiceCardProps {
  subscriptionId: string
  currency: string
}

export const UpcomingInvoiceCard = ({ subscriptionId, currency }: UpcomingInvoiceCardProps) => {
  const [expanded, setExpanded] = useState(false)

  const invoiceQuery = useQuery(
    getUpcomingInvoice,
    { subscriptionId },
    { enabled: Boolean(subscriptionId) }
  )

  if (invoiceQuery.isLoading) {
    return (
      <div className="bg-card rounded-lg border border-border shadow-sm mb-6 p-4">
        <Skeleton height={20} width={200} className="mb-2" />
        <Skeleton height={14} width={150} />
      </div>
    )
  }

  if (invoiceQuery.isError) {
    return (
      <div className="bg-card rounded-lg border border-border shadow-sm mb-6 p-4 text-sm text-muted-foreground">
        Failed to load upcoming invoice preview.
      </div>
    )
  }

  if (!invoiceQuery.data?.invoice) {
    return null
  }

  const invoice = invoiceQuery.data.invoice

  return (
    <div className="bg-card rounded-lg border border-border shadow-sm mb-6">
      <button
        className="w-full p-4 flex items-center justify-between cursor-pointer hover:bg-muted/5 transition-colors"
        onClick={() => setExpanded(!expanded)}
      >
        <div className="flex items-center gap-3">
          {expanded ? (
            <ChevronDown className="h-4 w-4 text-muted-foreground" />
          ) : (
            <ChevronRight className="h-4 w-4 text-muted-foreground" />
          )}
          <div className="text-left">
            <h3 className="text-md font-medium text-foreground">Upcoming Invoice</h3>
            <div className="text-xs text-muted-foreground mt-0.5">
              {parseAndFormatDate(invoice.periodStart)} &mdash;{' '}
              {parseAndFormatDate(invoice.periodEnd)}
              {invoice.lineItems.length > 0 && (
                <span className="ml-2">
                  &middot; {invoice.lineItems.length} line item
                  {invoice.lineItems.length !== 1 ? 's' : ''}
                </span>
              )}
            </div>
          </div>
        </div>
        <div className="text-right">
          <div className="text-lg font-semibold text-foreground tabular-nums">
            {formatCurrency(Number(invoice.total), currency)}
          </div>
          {invoice.amountDue !== invoice.total && (
            <div className="text-xs text-muted-foreground">
              Due: {formatCurrency(Number(invoice.amountDue), currency)}
            </div>
          )}
        </div>
      </button>

      {expanded && (
        <InvoiceDetails invoice={invoice} currency={currency} subscriptionId={subscriptionId} />
      )}
    </div>
  )
}

const InvoiceDetails = ({
  invoice,
  currency,
  subscriptionId,
}: {
  invoice: UpcomingInvoice
  currency: string
  subscriptionId: string
}) => {
  return (
    <div className="border-t border-border">
      {/* Metadata */}
      <div className="px-4 py-3 flex gap-6 text-xs text-muted-foreground border-b border-border bg-muted/5">
        <span>Invoice date: {parseAndFormatDate(invoice.invoiceDate)}</span>
        {invoice.dueDate && <span>Due: {parseAndFormatDate(invoice.dueDate)}</span>}
        <span>Net terms: {invoice.netTerms} days</span>
      </div>

      {/* Line items */}
      <div className="overflow-hidden">
        <table className="w-full">
          <thead className="border-b border-border">
            <tr>
              <th className="px-4 py-2 text-left text-xs font-medium text-muted-foreground">
                Item
              </th>
              <th className="px-4 py-2 text-right text-xs font-medium text-muted-foreground">
                Qty
              </th>
              <th className="px-4 py-2 text-right text-xs font-medium text-muted-foreground">
                Unit price
              </th>
              <th className="px-4 py-2 text-right text-xs font-medium text-muted-foreground">
                Subtotal
              </th>
            </tr>
          </thead>
          <tbody>
            {invoice.lineItems.map((line, idx) => (
              <LineItemRow
                key={line.id || idx}
                line={line}
                currency={currency}
                subscriptionId={subscriptionId}
                isLast={idx === invoice.lineItems.length - 1}
              />
            ))}
          </tbody>
        </table>
      </div>

      {/* Coupons */}
      {invoice.couponLineItems.length > 0 && (
        <div className="px-4 py-2 border-t border-border">
          {invoice.couponLineItems.map((coupon, idx) => (
            <div key={idx} className="flex justify-between text-sm py-1">
              <span className="text-muted-foreground">Coupon: {coupon.name}</span>
              <span className="text-success font-medium">
                -{formatCurrency(Number(coupon.total), currency)}
              </span>
            </div>
          ))}
        </div>
      )}

      {/* Totals */}
      <div className="px-4 py-3 border-t border-border bg-muted/5 space-y-1">
        <TotalRow label="Subtotal" amount={invoice.subtotal} currency={currency} />
        {invoice.discount > 0 && (
          <TotalRow label="Discount" amount={-invoice.discount} currency={currency} />
        )}
        {invoice.taxAmount > 0 && (
          <TotalRow label="Tax" amount={invoice.taxAmount} currency={currency} />
        )}
        <div className="flex justify-between text-sm font-semibold pt-1 border-t border-border">
          <span>Total</span>
          <span className="tabular-nums">{formatCurrency(Number(invoice.total), currency)}</span>
        </div>
        {invoice.appliedCredits > 0 && (
          <TotalRow label="Applied credits" amount={-invoice.appliedCredits} currency={currency} />
        )}
        {invoice.amountDue !== invoice.total && (
          <div className="flex justify-between text-sm font-semibold text-foreground">
            <span>Amount due</span>
            <span className="tabular-nums">
              {formatCurrency(Number(invoice.amountDue), currency)}
            </span>
          </div>
        )}
      </div>
    </div>
  )
}

const TotalRow = ({
  label,
  amount,
  currency,
}: {
  label: string
  amount: bigint | number
  currency: string
}) => (
  <div className="flex justify-between text-sm text-muted-foreground">
    <span>{label}</span>
    <span className="tabular-nums">{formatCurrency(Number(amount), currency)}</span>
  </div>
)

interface LineItemRowProps {
  line: UpcomingInvoice['lineItems'][number]
  currency: string
  subscriptionId: string
  isLast: boolean
}

const LineItemRow = ({ line, currency, subscriptionId, isLast }: LineItemRowProps) => {
  const [showUsage, setShowUsage] = useState(false)
  const [showSubLines, setShowSubLines] = useState(false)
  const hasMetric = Boolean(line.metricId)
  const hasSubLines = line.subLineItems.length > 0

  return (
    <>
      {/* Component header row */}
      <tr className={!isLast && !hasSubLines && !hasMetric ? 'border-b border-border/50' : ''}>
        <td className="px-4 pt-2 pb-1 text-sm" colSpan={hasSubLines ? 3 : 1}>
          <div className="flex items-center gap-1.5">
            <span className="font-medium text-foreground">{line.name}</span>
            {line.isProrated && (
              <span className="text-[10px] bg-muted text-muted-foreground px-1 rounded">
                prorated
              </span>
            )}
          </div>
          {line.startDate && line.endDate && (
            <div className="text-[11px] text-muted-foreground">
              {parseAndFormatDate(line.startDate)} &mdash; {parseAndFormatDate(line.endDate)}
            </div>
          )}
          {line.description && (
            <div className="text-[11px] text-muted-foreground">{line.description}</div>
          )}
        </td>
        {!hasSubLines && (
          <>
            <td className="px-4 pt-2 pb-1 text-sm text-right text-muted-foreground tabular-nums">
              {line.quantity ?? '-'}
            </td>
            <td className="px-4 pt-2 pb-1 text-sm text-right text-muted-foreground tabular-nums">
              {line.unitPrice ? formatCurrencyNoRounding(line.unitPrice, currency) : '-'}
            </td>
          </>
        )}
        <td className="px-4 pt-2 pb-1 text-sm text-right font-medium text-foreground tabular-nums">
          {formatCurrency(Number(line.subtotal), currency)}
        </td>
      </tr>

      {/* Subline toggle + rows */}
      {hasSubLines && (
        <>
          <tr>
            <td colSpan={4} className="px-4 pb-1">
              <button
                className="text-[11px] text-brand hover:underline cursor-pointer flex items-center gap-1"
                onClick={() => setShowSubLines(!showSubLines)}
              >
                <ChevronDown
                  className={`h-3 w-3 transition-transform ${showSubLines ? '' : '-rotate-90'}`}
                />
                {showSubLines ? 'Hide breakdown' : `${line.subLineItems.length} line items`}
              </button>
            </td>
          </tr>
          {showSubLines &&
            line.subLineItems.map((sub, idx) => (
              <tr key={idx}>
                <td className="pl-8 pr-4 py-0.5 text-[12px] text-muted-foreground">
                  {formatSubLineName(sub)}
                </td>
                <td
                  colSpan={2}
                  className="px-4 py-0.5 text-[12px] text-right text-muted-foreground/70 tabular-nums"
                >
                  {sub.quantity && sub.unitPrice
                    ? `${sub.quantity} × ${formatCurrencyNoRounding(sub.unitPrice, currency)}`
                    : ''}
                </td>
                <td className="px-4 py-0.5 text-[12px] text-right text-muted-foreground tabular-nums">
                  {formatCurrency(Number(sub.total), currency)}
                </td>
              </tr>
            ))}
        </>
      )}

      {/* Usage details toggle + chart */}
      {hasMetric && (
        <tr className={!isLast ? 'border-b border-border/50' : ''}>
          <td colSpan={4} className="px-4 pt-0.5 pb-2">
            <button
              className="text-[11px] text-brand hover:underline cursor-pointer flex items-center gap-1"
              onClick={() => setShowUsage(!showUsage)}
            >
              <ChevronDown
                className={`h-3 w-3 transition-transform ${showUsage ? '' : '-rotate-90'}`}
              />
              {showUsage ? 'Hide usage details' : 'Show usage details'}
            </button>
            {showUsage && line.metricId && (
              <div className="mt-2">
                <UsageBarChart
                  subscriptionId={subscriptionId}
                  metricId={line.metricId}
                  groupByDimensions={
                    Object.keys(line.groupByDimensions).length > 0
                      ? line.groupByDimensions
                      : undefined
                  }
                />
              </div>
            )}
          </td>
        </tr>
      )}

      {/* Bottom border for items without metric toggle */}
      {!hasMetric && hasSubLines && !isLast && (
        <tr className="border-b border-border/50">
          <td colSpan={4} className="h-0" />
        </tr>
      )}
    </>
  )
}

function formatSubLineName(sub: SubLineItem): string {
  if (sub.sublineAttributes.case === 'matrix') {
    const m = sub.sublineAttributes.value
    return m.dimension2Value ? `${m.dimension1Value} / ${m.dimension2Value}` : m.dimension1Value
  }
  return sub.name
}
