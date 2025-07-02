import { Badge } from '@md/ui'
import { match } from 'ts-pattern'

import { InvoiceStatus } from '@/rpc/api/invoices/v1/models_pb'

interface Props {
  status: InvoiceStatus
}

export const StatusPill = ({ status }: Props) =>
  match(status)
    .with(InvoiceStatus.VOID, () => <Badge variant="secondary">Void</Badge>)
    .with(InvoiceStatus.UNCOLLECTIBLE, () => <Badge variant="warning">Uncollectible</Badge>)
    .with(InvoiceStatus.FINALIZED, () => <Badge variant="success">Finalized</Badge>)
    .with(InvoiceStatus.DRAFT, () => <Badge variant="primary">Draft</Badge>)
    .otherwise(() => <Badge variant="destructive">Unknown</Badge>)
