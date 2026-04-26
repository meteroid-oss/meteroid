import { Badge } from '@md/ui'
import { match } from 'ts-pattern'

import { FeatureStatus } from '@/rpc/api/entitlements/v1/models_pb'

interface Props {
  status: FeatureStatus
}

export const FeatureStatusBadge = ({ status }: Props) =>
  match(status)
    .with(FeatureStatus.ACTIVE, () => <Badge variant="success">Active</Badge>)
    .with(FeatureStatus.DISABLED, () => <Badge variant="warning">Disabled</Badge>)
    .with(FeatureStatus.ARCHIVED, () => <Badge variant="ghost">Archived</Badge>)
    .otherwise(() => <Badge variant="destructive">Unknown</Badge>)
