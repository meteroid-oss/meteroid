import { Badge } from '@md/ui'
import { match } from 'ts-pattern'

import { FeatureStatus } from '@/rpc/api/entitlements/v1/models_pb'

interface Props {
  status: FeatureStatus
}

export const FeatureStatusBadge = ({ status }: Props) =>
  match(status)
    .with(FeatureStatus.ACTIVE, () => (
      <Badge variant="success" size="sm">
        Active
      </Badge>
    ))
    .with(FeatureStatus.DISABLED, () => (
      <Badge variant="warning" size="sm">
        Disabled
      </Badge>
    ))
    .with(FeatureStatus.ARCHIVED, () => (
      <Badge variant="ghost" size="sm">
        Archived
      </Badge>
    ))
    .otherwise(() => (
      <Badge variant="destructive" size="sm">
        Unknown
      </Badge>
    ))
