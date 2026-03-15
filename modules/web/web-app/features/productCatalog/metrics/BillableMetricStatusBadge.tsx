import { Badge } from '@md/ui'

interface Props {
  isArchived: boolean
  hasSyncError?: boolean
}

export const BillableMetricStatusBadge = ({ isArchived, hasSyncError = false }: Props) => {
  if (isArchived) return <Badge variant="secondary">Archived</Badge>
  if (hasSyncError) return <Badge variant="destructive">Error</Badge>
  return <Badge variant="success">Active</Badge>
}
