import { Badge, Tooltip, TooltipContent, TooltipTrigger } from '@md/ui'

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

/** A plan's draft version, beside its name: "Draft open" over a published one, else "Draft". */
export const DraftTag = ({ published }: { published: boolean }) =>
  published ? (
    <Tooltip>
      <TooltipTrigger asChild>
        <Badge variant="outline" size="sm" className="font-normal text-muted-foreground">
          Draft open
        </Badge>
      </TooltipTrigger>
      <TooltipContent>A new version is being drafted</TooltipContent>
    </Tooltip>
  ) : (
    <Badge variant="outline" size="sm" className="font-normal text-muted-foreground">
      Draft
    </Badge>
  )
