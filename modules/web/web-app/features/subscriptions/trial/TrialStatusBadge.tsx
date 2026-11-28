import { Badge } from '@md/ui'

import { TRIAL_STATUS } from './trialFacts'

import type { TrialConfig_Status } from '@/rpc/api/subscriptions/v1/models_pb'

export const TrialStatusBadge = ({ status }: { status: TrialConfig_Status }) => {
  const { label, variant } = TRIAL_STATUS[status]
  return <Badge variant={variant}>{label}</Badge>
}
