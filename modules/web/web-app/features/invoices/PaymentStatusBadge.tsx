import { Badge } from '@md/ui'
import { match } from 'ts-pattern'

import {
  InvoicePaymentStatus,
  Transaction,
  Transaction_PaymentStatusEnum,
} from '@/rpc/api/invoices/v1/models_pb'

interface Props {
  status: InvoicePaymentStatus
  /**
   * Fallback: a not-yet-paid invoice with a pending transaction (direct-debit in
   * flight) shows as "Processing" even if the backend has not stamped PROCESSING yet.
   */
  transactions?: Transaction[]
}

export const PaymentStatusBadge = ({ status, transactions }: Props) => {
  const hasPendingTransaction = transactions?.some(
    t => t.status === Transaction_PaymentStatusEnum.PENDING
  )

  if (
    hasPendingTransaction &&
    (status === InvoicePaymentStatus.UNPAID || status === InvoicePaymentStatus.PARTIALLY_PAID)
  ) {
    return <Badge variant="warning">Processing</Badge>
  }

  return match(status)
    .with(InvoicePaymentStatus.UNPAID, () => <Badge variant="secondary">Unpaid</Badge>)
    .with(InvoicePaymentStatus.PARTIALLY_PAID, () => (
      <Badge variant="warning">Partially Paid</Badge>
    ))
    .with(InvoicePaymentStatus.PAID, () => <Badge variant="success">Paid</Badge>)
    .with(InvoicePaymentStatus.PROCESSING, () => <Badge variant="warning">Processing</Badge>)
    .otherwise(() => <Badge variant="destructive">Unknown</Badge>)
}
