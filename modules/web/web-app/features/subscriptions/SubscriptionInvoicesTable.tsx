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
import { ReceiptText, RefreshCw, TriangleAlert } from 'lucide-react'
import { useState } from 'react'
import { Link } from 'react-router'

import { SectionEmptyState } from '@/components/empty-state/SectionEmptyState'
import { ContainedTableSection } from '@/features/customers/cards/ContainedTableSection'
import { InvoiceStateBadge } from '@/features/invoices/InvoiceStatusBadge'
import { invoiceRowState } from '@/features/invoices/invoiceState'
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

  // The rows laid out as the customer page's tables: 24px in on the left, 32px column headers.
  const tableClass =
    '[&_td:first-child]:pl-6 [&_td:last-child]:pr-6 [&_td]:px-2.5 [&_th:first-child]:pl-6 [&_th:last-child]:pr-6 [&_th]:!h-8 [&_th]:px-2.5 [&_th]:text-[13px] [&_th]:font-medium [&_th]:text-foreground/60 [&_tbody_tr]:h-[52px] [&_td]:text-[13px]'

  return (
    <ContainedTableSection
      title="Invoices"
      headerExtra={
        <>
          <Link
            to={`${basePath}/invoices?subscription=${encodeURIComponent(subscriptionLocalId)}`}
            className="text-[13px] text-muted-foreground hover:text-foreground"
          >
            All invoices →
          </Link>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => query.refetch()}
            disabled={query.isFetching}
            className="h-7 w-7 p-0 text-muted-foreground hover:text-foreground"
            title="Refresh invoices"
            aria-label="Refresh invoices"
          >
            <RefreshCw className={`h-3.5 w-3.5 ${query.isFetching ? 'animate-spin' : ''}`} />
          </Button>
        </>
      }
    >
      {query.isLoading ? (
        <div className="p-4">
          <Skeleton height={120} />
        </div>
      ) : query.isError ? (
        <SectionEmptyState
          framed={false}
          icon={<TriangleAlert />}
          title="Invoices could not be loaded"
          action={
            <Button size="sm" variant="secondary" onClick={() => query.refetch()}>
              Try again
            </Button>
          }
        >
          Something went wrong while fetching this subscription&apos;s invoices.
        </SectionEmptyState>
      ) : invoices.length === 0 ? (
        <SectionEmptyState framed={false} icon={<ReceiptText />} title="No invoices yet">
          {firstInvoiceDate
            ? `The first invoice is issued on ${formatDateOnly(firstInvoiceDate)}. Invoices for this subscription appear here as they are issued.`
            : 'Invoices for this subscription appear here as they are issued.'}
        </SectionEmptyState>
      ) : (
        <>
          <Table className={tableClass}>
            <TableHeader className="border-t-0 bg-transparent">
              <TableRow className="hover:bg-transparent">
                <TableHead>Date</TableHead>
                <TableHead>Invoice</TableHead>
                <TableHead className="text-right">Total</TableHead>
                {hasMinimum && <TableHead className="text-right">Shortfall</TableHead>}
                {anyCredit && <TableHead className="text-right">Paid with credit</TableHead>}
                <TableHead className="text-right">
                  {chargeAutomatically ? 'Charged' : 'Billed'}
                </TableHead>
                <TableHead>Status</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {invoices.map(invoice => {
                const money = (minor: bigint) => formatCurrency(minor, invoice.currency)
                return (
                  <TableRow key={invoice.id} className="hover:bg-foreground/[0.04]">
                    <TableCell className="text-muted-foreground">
                      {formatDateOnly(invoice.invoiceDate)}
                    </TableCell>
                    <TableCell>
                      <Link
                        to={`${basePath}/invoices/${invoice.id}`}
                        className="font-medium text-foreground hover:underline"
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
                    <TableCell>
                      <InvoiceStateBadge invoice={invoice} now={now} />
                    </TableCell>
                  </TableRow>
                )
              })}
            </TableBody>
          </Table>
          {totalItems > invoices.length && (
            // More than fit: load the next few, as before.
            <div className="flex h-10 items-center justify-between border-t border-border pl-6 pr-2 text-xs text-muted-foreground">
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
    </ContainedTableSection>
  )
}
