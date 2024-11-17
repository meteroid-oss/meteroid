import { PlanStatus, PlanType } from '@/rpc/api/plans/v1/models_pb'
import { Badge } from '@ui/components'
import { CheckIcon, CircleDashed, XIcon } from 'lucide-react'

export function displayPlanType(planType: PlanType) {
  switch (planType) {
    case PlanType.FREE:
      return <Badge variant="primary">Free</Badge>
    case PlanType.STANDARD:
      return <Badge variant="brand">Standard</Badge>
    case PlanType.CUSTOM:
      return <Badge variant="destructive">Custom</Badge>
    default:
      return '-'
  }
}

export function displayPlanStatus(status: PlanStatus) {
  switch (status) {
    case PlanStatus.ACTIVE:
      return <CheckIcon size="12" />
    case PlanStatus.DRAFT:
      return <CircleDashed size="12" />
    case PlanStatus.INACTIVE:
      return <XIcon size="12" />
    case PlanStatus.ARCHIVED:
      return <XIcon size="12" />
    default:
      return '-'
  }
}

export function printPlanStatus(status: PlanStatus) {
  switch (status) {
    case PlanStatus.ACTIVE:
      return 'Active'
    case PlanStatus.DRAFT:
      return 'Draft'
    case PlanStatus.INACTIVE:
      return 'Inactive'
    case PlanStatus.ARCHIVED:
      return 'Archived'
    default:
      return undefined
  }
}
