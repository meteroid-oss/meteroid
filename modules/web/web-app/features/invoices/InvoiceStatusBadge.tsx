import { Badge } from '@md/ui'
import { match } from 'ts-pattern'

import { PaymentStatusBadge } from '@/features/invoices/PaymentStatusBadge'
import { invoiceRowState, openLabel, overdueLabel } from '@/features/invoices/invoiceState'
import { InvoiceStatus } from '@/rpc/api/invoices/v1/models_pb'

import type { Invoice } from '@/rpc/api/invoices/v1/models_pb'

interface Props {
  status: InvoiceStatus
  // When set, this invoice was merged into a consolidated parent; show "Merged" instead of its
  // (permanently Draft) own status.
  consolidatedInto?: string
}

export const InvoiceStatusBadge = ({ status, consolidatedInto }: Props) =>
  consolidatedInto ? (
    <Badge variant="secondary">Merged</Badge>
  ) : (
    match(status)
      .with(InvoiceStatus.VOID, () => <Badge variant="secondary">Void</Badge>)
      .with(InvoiceStatus.UNCOLLECTIBLE, () => <Badge variant="warning">Uncollectible</Badge>)
      .with(InvoiceStatus.FINALIZED, () => <Badge variant="success">Finalized</Badge>)
      .with(InvoiceStatus.DRAFT, () => <Badge variant="ghost">Draft</Badge>)
      .with(InvoiceStatus.CLOSED, () => <Badge variant="ghost">Closed</Badge>)
      .otherwise(() => <Badge variant="destructive">Unknown</Badge>)
  )

/** Owed invoices read Overdue · N days or Open · due; others as their payment or own status. */
export const InvoiceStateBadge = ({ invoice, now }: { invoice: Invoice; now: Date }) => {
  const state = invoiceRowState(invoice, now)
  if (state.kind === 'overdue') {
    return <Badge variant="destructive">{overdueLabel(state.days)}</Badge>
  }
  if (state.kind === 'open') {
    return <Badge variant="secondary">{openLabel(state.due)}</Badge>
  }
  return invoice.status === InvoiceStatus.FINALIZED && !invoice.consolidatedIntoInvoiceId ? (
    <PaymentStatusBadge
      status={invoice.paymentStatus}
      settledByCreditNotes={invoice.settledByCreditNotes}
    />
  ) : (
    <InvoiceStatusBadge
      status={invoice.status}
      consolidatedInto={invoice.consolidatedIntoInvoiceId}
    />
  )
}

/** The payment status, except that an overdue invoice reads Overdue · N days. */
export const InvoicePaymentBadge = ({ invoice, now }: { invoice: Invoice; now: Date }) => {
  const state = invoiceRowState(invoice, now)
  return state.kind === 'overdue' ? (
    <Badge variant="destructive">{overdueLabel(state.days)}</Badge>
  ) : (
    <PaymentStatusBadge
      status={invoice.paymentStatus}
      settledByCreditNotes={invoice.settledByCreditNotes}
    />
  )
}
