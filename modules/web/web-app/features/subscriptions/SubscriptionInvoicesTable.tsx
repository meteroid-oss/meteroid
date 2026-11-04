import {
  Button,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { RefreshCw } from 'lucide-react'
import { useState } from 'react'
import { Link } from 'react-router-dom'

import { InvoiceStateBadge } from '@/features/invoices/InvoiceStatusBadge'
import { invoiceRowState } from '@/features/invoices/invoiceState'
import { SubscriptionSectionCard } from '@/features/subscriptions/SubscriptionSectionCard'
import { invoiceAfterCredit } from '@/features/subscriptions/subscriptionOverview'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { formatDateOnly } from '@/lib/utils/date'
import { formatCurrency } from '@/lib/utils/numbers'
import { listInvoices } from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'

import type { Invoice } from '@/rpc/api/invoices/v1/models_pb'

const PAGE_SIZE = 6

const AfterCredit = ({ invoice, now }: { invoice: Invoice; now: Date }) => {
  const money = (minor: bigint) => formatCurrency(minor, invoice.currency)
  const after = invoiceAfterCredit(invoice)
  switch (after.kind) {
    case 'none':
      return <>—</>
    case 'draft':
      return <span className="text-muted-foreground">{money(after.amount)} · draft</span>
    case 'settled':
      return <>{money(after.amount)}</>
    case 'due': {
      const overdue = invoiceRowState(invoice, now).kind === 'overdue'
      return <span className={overdue ? 'text-warning' : undefined}>{money(after.amount)} due</span>
    }
  }
}

export const SubscriptionInvoicesTable = ({
  subscriptionLocalId,
  chargeAutomatically,
  hasMinimum,
  firstInvoiceDate,
}: {
  subscriptionLocalId: string
  chargeAutomatically: boolean
  /** Shows the minimum-commitment shortfall each invoice's total includes. */
  hasMinimum: boolean
  /** When no invoice exists yet, the date the first one is issued. */
  firstInvoiceDate?: string
}) => {
  const basePath = useBasePath()
  const [now] = useState(() => new Date())
  const [limit, setLimit] = useState(PAGE_SIZE)
  const query = useQuery(listInvoices, {
    subscriptionId: subscriptionLocalId,
    pagination: { perPage: limit, page: 0 },
  })

  const invoices = query.data?.invoices ?? []
  const totalItems = query.data?.paginationMeta?.totalItems ?? invoices.length
  const anyCredit = invoices.some(i => i.appliedCredits > 0n)

  return (
    <SubscriptionSectionCard
      title="Invoices"
      aside={
        <div className="flex items-center gap-3">
          <Link
            to={`${basePath}/invoices?subscription=${encodeURIComponent(subscriptionLocalId)}`}
            className="text-sm text-brand hover:underline"
          >
            All invoices →
          </Link>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => query.refetch()}
            disabled={query.isFetching}
            className="h-7 w-7 p-0"
            title="Refresh invoices"
          >
            <RefreshCw className={`h-3.5 w-3.5 ${query.isFetching ? 'animate-spin' : ''}`} />
          </Button>
        </div>
      }
    >
      <div className="p-4">
        {query.isLoading ? (
          <Skeleton height={120} />
        ) : query.isError ? (
          <p className="text-sm text-muted-foreground">The invoices could not be loaded.</p>
        ) : invoices.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {firstInvoiceDate
              ? `No invoices yet · the first is issued on ${formatDateOnly(firstInvoiceDate)}`
              : 'No invoices yet'}
          </p>
        ) : (
          <>
            <Table>
              <TableHeader className="border-t-0 bg-transparent">
                <TableRow className="hover:bg-transparent">
                  <TableHead className="pl-0">Date</TableHead>
                  <TableHead>Invoice</TableHead>
                  <TableHead className="text-right">Total</TableHead>
                  {hasMinimum && <TableHead className="text-right">Shortfall</TableHead>}
                  {anyCredit && <TableHead className="text-right">Paid with credit</TableHead>}
                  <TableHead className="text-right">
                    {chargeAutomatically ? 'Charged' : 'Billed'}
                  </TableHead>
                  <TableHead className="pr-0">Status</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {invoices.map(invoice => {
                  const money = (minor: bigint) => formatCurrency(minor, invoice.currency)
                  return (
                    <TableRow key={invoice.id}>
                      <TableCell className="pl-0 text-muted-foreground">
                        {formatDateOnly(invoice.invoiceDate)}
                      </TableCell>
                      <TableCell>
                        <Link
                          to={`${basePath}/invoices/${invoice.id}`}
                          className="font-medium text-brand hover:underline"
                        >
                          {invoice.consolidatedIntoInvoiceId
                            ? `Merged into ${invoice.consolidatedIntoInvoiceNumber ?? 'consolidated invoice'}`
                            : invoice.invoiceNumber}
                        </Link>
                      </TableCell>
                      <TableCell className="text-right tabular-nums">
                        {money(invoice.total)}
                      </TableCell>
                      {hasMinimum && (
                        <TableCell className="text-right tabular-nums text-muted-foreground">
                          {invoice.minimumShortfall > 0n ? money(invoice.minimumShortfall) : '—'}
                        </TableCell>
                      )}
                      {anyCredit && (
                        <TableCell className="text-right tabular-nums text-muted-foreground">
                          {invoice.appliedCredits > 0n ? `−${money(invoice.appliedCredits)}` : '—'}
                        </TableCell>
                      )}
                      <TableCell className="text-right tabular-nums">
                        <AfterCredit invoice={invoice} now={now} />
                      </TableCell>
                      <TableCell className="pr-0">
                        <InvoiceStateBadge invoice={invoice} now={now} />
                      </TableCell>
                    </TableRow>
                  )
                })}
              </TableBody>
            </Table>
            {totalItems > invoices.length && (
              <div className="mt-3 flex items-center justify-between text-xs text-muted-foreground">
                <span>
                  {invoices.length} of {totalItems} invoices
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={query.isFetching}
                  onClick={() => setLimit(limit + PAGE_SIZE)}
                >
                  Show more
                </Button>
              </div>
            )}
          </>
        )}
      </div>
    </SubscriptionSectionCard>
  )
}
