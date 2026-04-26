/**
 * EntitlementSpecDialog — EntitlementDialog wrapper for creation wizard flows.
 * Converts the submitted form values into a PendingEntitlementSpec and returns it
 * to the caller via onAdd. No API calls — specs are resolved later in batch by resolveEntitlementSpecs.
 */
import { EntitlementDialog, EntitlementFormValues } from '@/features/entitlements/EntitlementDialog'
import { useQuery } from '@/lib/connectrpc'
import { listFeatures } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'

import type { PendingEntitlementSpec } from './types'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  onAdd: (spec: PendingEntitlementSpec) => void
  initialSpec?: PendingEntitlementSpec
  existingEntitlements?: PendingEntitlementSpec[]
}

export function EntitlementSpecDialog({ open, onOpenChange, onAdd, initialSpec, existingEntitlements }: Props) {
  const featuresQuery = useQuery(listFeatures, { pagination: { page: 0, perPage: 100 }, statuses: [] })
  const features = featuresQuery.data?.features ?? []

  const isEdit = initialSpec !== undefined

  const existingFeatureIds = new Set(
    (existingEntitlements ?? []).filter(e => e.featureId).map(e => e.featureId!)
  )

  const lockedFeature =
    isEdit && initialSpec.featureId
      ? {
          id: initialSpec.featureId,
          name: initialSpec.featureDisplayName,
          isMetered: initialSpec.featureType === 'metered',
        }
      : undefined

  const initialValues: Partial<EntitlementFormValues> | undefined = initialSpec
    ? {
        featureId: initialSpec.featureId,
        featureName: initialSpec.featureName,
        featureType: initialSpec.featureType,
        metricId: initialSpec.metricId,
        boolEnabled: initialSpec.boolEnabled,
        limit: initialSpec.limit,
        resetPeriodType: initialSpec.resetPeriodType,
        resetUnit: initialSpec.resetUnit,
        resetInterval: initialSpec.resetInterval,
        overageBehaviorType: initialSpec.overageBehaviorType,
        gracePeriodPct: initialSpec.gracePeriodPct,
        warningThresholdPct: initialSpec.warningThresholdPct,
        meteredEnabled: initialSpec.meteredEnabled,
      }
    : undefined

  const handleSubmit = (data: EntitlementFormValues) => {
    const selectedFeature = features.find(f => f.id === data.featureId)
    const featureDisplayName =
      data.featureName ?? selectedFeature?.name ?? data.featureId ?? ''

    onAdd({
      featureId: data.featureId,
      featureName: data.featureName,
      featureDisplayName,
      featureType: data.featureType,
      metricId: data.metricId,
      boolEnabled: data.boolEnabled,
      limit: data.limit,
      resetPeriodType: data.resetPeriodType ?? 'never',
      resetUnit: data.resetUnit,
      resetInterval: data.resetInterval,
      overageBehaviorType: data.overageBehaviorType,
      gracePeriodPct: data.gracePeriodPct,
      warningThresholdPct: data.warningThresholdPct,
      meteredEnabled: data.meteredEnabled,
      productId: selectedFeature?.product?.id,
      productName: selectedFeature?.product?.name,
    })
  }

  return (
    <EntitlementDialog
      open={open}
      onOpenChange={onOpenChange}
      onSubmit={handleSubmit}
      initialValues={initialValues}
      lockedFeature={lockedFeature}
      existingFeatureIds={existingFeatureIds}
      title={isEdit ? 'Edit entitlement' : 'Add entitlement'}
      submitLabel={isEdit ? 'Save changes' : 'Add entitlement'}
    />
  )
}
