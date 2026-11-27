import { Badge } from '@md/ui'

import { QuoteApprovalStatus, QuoteStatus } from '@/rpc/api/quotes/v1/models_pb'

interface Props {
  status: QuoteStatus
}

export const QuoteStatusBadge = ({ status }: Props) => {
  switch (status) {
    case QuoteStatus.DRAFT:
      // The platform's grey tag, at the design's 20px (11px text, 6px sides).
      return (
        <Badge variant="secondary" size="sm" className="h-5 px-1.5">
          Draft
        </Badge>
      )
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

export const QuoteApprovalBadge = ({ status }: { status: QuoteApprovalStatus }) => {
  switch (status) {
    case QuoteApprovalStatus.APPROVAL_PENDING:
      return <Badge variant="warning">Approval pending</Badge>
    case QuoteApprovalStatus.APPROVAL_APPROVED:
      return <Badge variant="success">Approved</Badge>
    case QuoteApprovalStatus.APPROVAL_REJECTED:
      return <Badge variant="destructive">Approval rejected</Badge>
    default:
      return null
  }
}

export const ChangesRequestedBadge = () => <Badge variant="warning">Changes requested</Badge>
