import { Badge } from '@md/ui'

import { PlanStatus } from '@/rpc/api/plans/v1/models_pb'

interface Props {
  status: PlanStatus
}

export const PlanStatusBadge = ({ status }: Props) => {
  switch (status) {
    case PlanStatus.ACTIVE:
      return <Badge variant="success">Active</Badge>
    case PlanStatus.DRAFT:
      return <Badge variant="ghost">Draft</Badge>
    case PlanStatus.INACTIVE:
      return <Badge variant="secondary">Inactive</Badge>
    case PlanStatus.ARCHIVED:
      return <Badge variant="secondary">Archived</Badge>
    default:
      return null
  }
}
