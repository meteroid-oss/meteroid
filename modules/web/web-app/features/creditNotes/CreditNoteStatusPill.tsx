import { Badge } from '@md/ui'
import { match } from 'ts-pattern'

import { CreditNoteStatus } from '@/rpc/api/creditnotes/v1/models_pb'

interface Props {
  status: CreditNoteStatus
}

export const CreditNoteStatusPill = ({ status }: Props) =>
  match(status)
    .with(CreditNoteStatus.VOIDED, () => <Badge variant="secondary">Voided</Badge>)
    .with(CreditNoteStatus.FINALIZED, () => <Badge variant="success">Finalized</Badge>)
    .with(CreditNoteStatus.DRAFT, () => <Badge variant="primary">Draft</Badge>)
    .otherwise(() => <Badge variant="destructive">Unknown</Badge>)
