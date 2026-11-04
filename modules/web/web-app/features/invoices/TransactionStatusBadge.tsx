import { Badge } from '@md/ui'
import { match } from 'ts-pattern'

import { reversedLabel } from '@/features/invoices/refunds'
import { Transaction, Transaction_PaymentStatusEnum } from '@/rpc/api/invoices/v1/models_pb'

interface Props {
  status: Transaction_PaymentStatusEnum
  /** Words a clawed-back payment by how the money left: a dispute is not a refund. */
  transaction?: Pick<Transaction, 'reversalKind' | 'amountDisputed'>
}

export const TransactionStatusBadge = ({ status, transaction }: Props) =>
  match(status)
    .with(Transaction_PaymentStatusEnum.READY, () => <Badge variant="secondary">Ready</Badge>)
    .with(Transaction_PaymentStatusEnum.PENDING, () => <Badge variant="warning">Pending</Badge>)
    .with(Transaction_PaymentStatusEnum.SETTLED, () => <Badge variant="success">Settled</Badge>)
    .with(Transaction_PaymentStatusEnum.CANCELLED, () => (
      <Badge variant="secondary">Cancelled</Badge>
    ))
    .with(Transaction_PaymentStatusEnum.FAILED, () => <Badge variant="destructive">Failed</Badge>)
    .with(Transaction_PaymentStatusEnum.REFUNDED, () => {
      const label = reversedLabel(transaction)
      return <Badge variant={label === 'Charged back' ? 'warning' : 'secondary'}>{label}</Badge>
    })
    .otherwise(() => <Badge variant="destructive">Unknown</Badge>)
