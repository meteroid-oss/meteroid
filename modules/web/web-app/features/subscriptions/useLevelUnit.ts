import { metricUnitFromProto } from '@/features/billing/metrics/units'
import { unitIsCurrency } from '@/features/plans/charges/kinds'
import { useComponentMetric } from '@/features/plans/pricecomponents/componentRefs'

import type { LevelUnit } from '@/features/subscriptions/capacityLevels'
import type { SubscriptionFee } from '@/rpc/api/subscriptions/v1/models_pb'

/** How a usage-levels charge's sizes read: its metric's unit, or money in `currency`. */
export const useLevelUnit = (fee: SubscriptionFee | undefined, currency: string): LevelUnit => {
  const metricId = fee?.fee.case === 'capacity' ? fee.fee.value.metricId : undefined
  const unit = metricUnitFromProto(useComponentMetric(metricId))
  return { money: unitIsCurrency(unit, currency), plural: unit?.plural }
}
