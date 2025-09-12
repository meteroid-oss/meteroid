import { Badge } from '@md/ui'
import { match } from 'ts-pattern'

import { Transaction_PaymentStatusEnum } from '@/rpc/api/invoices/v1/models_pb'

interface Props {
  status: Transaction_PaymentStatusEnum
}

export const TransactionStatusBadge = ({ status }: Props) =>
  match(status)
    .with(Transaction_PaymentStatusEnum.READY, () => <Badge variant="secondary">Ready</Badge>)
    .with(Transaction_PaymentStatusEnum.PENDING, () => <Badge variant="warning">Pending</Badge>)
    .with(Transaction_PaymentStatusEnum.SETTLED, () => <Badge variant="success">Settled</Badge>)
    .with(Transaction_PaymentStatusEnum.CANCELLED, () => <Badge variant="secondary">Cancelled</Badge>)
    .with(Transaction_PaymentStatusEnum.FAILED, () => <Badge variant="destructive">Failed</Badge>)
    .otherwise(() => <Badge variant="destructive">Unknown</Badge>)