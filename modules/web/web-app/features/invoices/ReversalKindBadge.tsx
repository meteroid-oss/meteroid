import { Badge, Tooltip, TooltipContent, TooltipTrigger } from '@md/ui'

import { REVERSAL_KIND_LABELS } from '@/features/invoices/refunds'
import { Transaction_ReversalKind } from '@/rpc/api/invoices/v1/models_pb'

interface Props {
  kind: Transaction_ReversalKind
  reason?: string
}

export const ReversalKindBadge = ({ kind, reason }: Props) => {
  const badge = (
    <Badge variant="destructive" size="sm">
      {REVERSAL_KIND_LABELS[kind] ?? 'Reversed'}
    </Badge>
  )

  if (!reason) return badge

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span>{badge}</span>
      </TooltipTrigger>
      <TooltipContent className="max-w-xs">{reason}</TooltipContent>
    </Tooltip>
  )
}
