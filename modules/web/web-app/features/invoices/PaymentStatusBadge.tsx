import { Badge } from '@md/ui'
import { match } from 'ts-pattern'

import { InvoicePaymentStatus } from '@/rpc/api/invoices/v1/models_pb'

interface Props {
  status: InvoicePaymentStatus
}

export const PaymentStatusBadge = ({ status }: Props) =>
  match(status)
    .with(InvoicePaymentStatus.UNPAID, () => <Badge variant="secondary">Unpaid</Badge>)
    .with(InvoicePaymentStatus.PARTIALLY_PAID, () => <Badge variant="warning">Partially Paid</Badge>)
    .with(InvoicePaymentStatus.PAID, () => <Badge variant="success">Paid</Badge>)
    .otherwise(() => <Badge variant="destructive">Unknown</Badge>)