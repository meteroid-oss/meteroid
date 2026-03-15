import { Badge } from '@md/ui'

type RowStatus = 'active' | 'missing' | 'orphaned'

interface Props {
  status: RowStatus
}

export const MatrixRowStatusBadge = ({ status }: Props) => {
  switch (status) {
    case 'active':
      return <Badge variant="default">Active</Badge>
    case 'missing':
      return <Badge variant="outline">Missing</Badge>
    case 'orphaned':
      return <Badge variant="destructive">Orphaned</Badge>
  }
}
