import { Badge } from '@md/ui'

import { QuoteStatus } from '@/rpc/api/quotes/v1/models_pb'

interface Props {
  status: QuoteStatus
}

export const QuoteStatusBadge = ({ status }: Props) => {
  switch (status) {
    case QuoteStatus.DRAFT:
      return <Badge variant="ghost">Draft</Badge>
    case QuoteStatus.PENDING:
      return <Badge variant="warning">Pending</Badge>
    case QuoteStatus.ACCEPTED:
      return <Badge variant="success">Accepted</Badge>
    case QuoteStatus.DECLINED:
      return <Badge variant="destructive">Declined</Badge>
    case QuoteStatus.EXPIRED:
      return <Badge variant="outline">Expired</Badge>
    case QuoteStatus.CANCELLED:
      return <Badge variant="outline">Cancelled</Badge>
    default:
      return null
  }
}
