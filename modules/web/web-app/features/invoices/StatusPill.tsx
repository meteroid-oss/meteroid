import { Pill } from '@ui/components'
import { match } from 'ts-pattern'

import { InvoiceStatus } from '@/rpc/api/invoices/v1/models_pb'

interface Props {
  status: InvoiceStatus
}

export const StatusPill = ({ status }: Props) =>
  match(status)
    .with(InvoiceStatus.VOID, () => <Pill color="neutral">Void</Pill>)
    .with(InvoiceStatus.PENDING, () => <Pill color="warning">Pending</Pill>)
    .with(InvoiceStatus.FINALIZED, () => <Pill color="success">Finalized</Pill>)
    .with(InvoiceStatus.DRAFT, () => <Pill color="blue">Draft</Pill>)
    .otherwise(() => <Pill color="danger">Unknown</Pill>)
