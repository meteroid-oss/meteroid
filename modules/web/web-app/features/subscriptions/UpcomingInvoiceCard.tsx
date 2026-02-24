import { Skeleton } from '@md/ui'
import { ChevronDown, ChevronRight } from 'lucide-react'
import { useState } from 'react'

import { useQuery } from '@/lib/connectrpc'
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

  if (invoiceQuery.isError || !invoiceQuery.data?.invoice) {
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
        {invoice.planName && <span>Plan: {invoice.planName}</span>}
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
                isEven={idx % 2 === 0}
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
  isEven: boolean
}

const LineItemRow = ({ line, currency, subscriptionId, isEven }: LineItemRowProps) => {
  const [showUsage, setShowUsage] = useState(false)
  const hasMetric = Boolean(line.metricId)

  return (
    <>
      <tr className={isEven ? 'bg-card' : 'bg-muted/10'}>
        <td className="px-4 py-2 text-sm">
          <div className="flex items-center gap-1">
            <span className="font-medium text-foreground">{line.name}</span>
            {line.isProrated && (
              <span className="text-[10px] bg-muted text-muted-foreground px-1 rounded">
                prorated
              </span>
            )}
            {hasMetric && (
              <button
                className="text-[10px] text-brand hover:underline ml-1 cursor-pointer"
                onClick={() => setShowUsage(!showUsage)}
              >
                {showUsage ? 'hide usage' : 'usage'}
              </button>
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
          {/* Sub line items */}
          {line.subLineItems.length > 0 && (
            <div className="mt-1 space-y-0.5">
              {line.subLineItems.map((sub, idx) => (
                <div key={idx} className="text-[11px] text-muted-foreground flex gap-4">
                  <span>{sub.name}</span>
                  {sub.quantity && <span>qty: {sub.quantity}</span>}
                  <span>{formatCurrency(Number(sub.total), currency)}</span>
                </div>
              ))}
            </div>
          )}
        </td>
        <td className="px-4 py-2 text-sm text-right text-muted-foreground tabular-nums">
          {line.quantity ?? '-'}
        </td>
        <td className="px-4 py-2 text-sm text-right text-muted-foreground tabular-nums">
          {line.unitPrice ? formatCurrencyNoRounding(line.unitPrice, currency) : '-'}
        </td>
        <td className="px-4 py-2 text-sm text-right font-medium text-foreground tabular-nums">
          {formatCurrency(Number(line.subtotal), currency)}
        </td>
      </tr>
      {showUsage && line.metricId && (
        <tr>
          <td colSpan={4} className="px-4 py-3 bg-muted/5">
            <UsageBarChart subscriptionId={subscriptionId} metricId={line.metricId} />
          </td>
        </tr>
      )}
    </>
  )
}
