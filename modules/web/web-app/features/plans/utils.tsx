import { Badge } from '@ui/components'
import { AlertTriangle, CheckIcon, LockIcon } from 'lucide-react'

import { PlanStatus, PlanType } from '@/rpc/api/plans/v1/models_pb'

export function displayPlanType(planType: PlanType) {
  switch (planType) {
    case PlanType.FREE:
      return <Badge variant="outline">Free</Badge>
    case PlanType.STANDARD:
      return <Badge variant="outline">Standard</Badge>
    case PlanType.CUSTOM:
      return <Badge variant="outline">Custom</Badge>
    default:
      return '-'
  }
}

export function displayPlanStatus(status: PlanStatus) {
  switch (status) {
    case PlanStatus.ACTIVE:
      return <CheckIcon size="14" />
    case PlanStatus.DRAFT:
      return (
        <>
          <AlertTriangle size="14" />
        </>
      )
    case PlanStatus.INACTIVE:
      return <LockIcon size="14" />
    case PlanStatus.ARCHIVED:
      return <LockIcon size="14" />
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
