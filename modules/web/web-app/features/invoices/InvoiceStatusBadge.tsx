import { Badge } from '@md/ui'
import { match } from 'ts-pattern'

import { InvoiceStatus } from '@/rpc/api/invoices/v1/models_pb'

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
      .otherwise(() => <Badge variant="destructive">Unknown</Badge>)
  )

