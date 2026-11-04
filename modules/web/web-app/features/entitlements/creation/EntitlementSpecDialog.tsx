/**
 * EntitlementSpecDialog — EntitlementDialog wrapper for creation wizard flows.
 * Converts the submitted form values into a PendingEntitlementSpec and returns it
 * to the caller via onAdd. No API calls — specs are resolved later in batch by resolveEntitlementSpecs.
 */
import { EntitlementDialog, EntitlementFormValues } from '@/features/entitlements/EntitlementDialog'

import type { PendingEntitlementSpec } from './types'

interface Props {
  open: boolean
  onOpenChange: (open: boolean) => void
  onAdd: (spec: PendingEntitlementSpec) => void
  initialSpec?: PendingEntitlementSpec
  existingEntitlements?: PendingEntitlementSpec[]
}

export function EntitlementSpecDialog({ open, onOpenChange, onAdd, initialSpec, existingEntitlements }: Props) {
  const isEdit = initialSpec !== undefined

  const existingFeatureIds = new Set(
    (existingEntitlements ?? []).filter(e => e.featureId).map(e => e.featureId!)
  )

  const lockedFeature =
    isEdit && initialSpec.featureId
      ? {
          id: initialSpec.featureId,
          name: initialSpec.featureDisplayName,
          featureType: initialSpec.featureType,
        }
      : undefined

  const initialValues: Partial<EntitlementFormValues> | undefined = initialSpec
    ? {
        featureId: initialSpec.featureId,
        featureName: initialSpec.featureName,
        featureCode: initialSpec.featureCode,
        featureType: initialSpec.featureType,
        metricId: initialSpec.metricId,
        boolEnabled: initialSpec.boolEnabled,
        limit: initialSpec.limit,
        resetPeriodType: initialSpec.resetPeriodType,
        resetUnit: initialSpec.resetUnit,
        resetInterval: initialSpec.resetInterval,
        meteredEnabled: initialSpec.meteredEnabled,
        groupBy: initialSpec.groupBy,
        overageType: initialSpec.overageType,
        gracePct: initialSpec.gracePct,
        thresholds: initialSpec.thresholds,
        configKind: initialSpec.configKind,
        configNumber: initialSpec.configNumber,
        configBoolean: initialSpec.configBoolean,
        configText: initialSpec.configText,
        configJson: initialSpec.configJson,
        configEntries: initialSpec.configEntries,
        configOptions: initialSpec.configOptions,
      }
    : undefined

  const handleSubmit = (data: EntitlementFormValues) => {
    const featureDisplayName =
      data.featureName ??
      data.selectedFeature?.name ??
      (isEdit ? initialSpec.featureDisplayName : undefined) ??
      data.featureId ??
      ''

    onAdd({
      featureId: data.featureId,
      featureName: data.featureName,
      featureCode: data.featureCode,
      featureDisplayName,
      featureType: data.featureType,
      metricId: data.metricId,
      boolEnabled: data.boolEnabled,
      limit: data.limit,
      resetPeriodType: data.resetPeriodType ?? 'never',
      resetUnit: data.resetUnit,
      resetInterval: data.resetInterval,
      meteredEnabled: data.meteredEnabled,
      groupBy: data.groupBy?.trim() || undefined,
      overageType: data.overageType,
      gracePct: data.gracePct,
      thresholds: data.thresholds,
      configKind: data.configKind,
      configNumber: data.configNumber,
      configBoolean: data.configBoolean,
      configText: data.configText,
      configJson: data.configJson,
      configEntries: data.configEntries,
      configOptions: data.configOptions,
      productId: data.selectedFeature?.product?.id ?? (isEdit ? initialSpec.productId : undefined),
      productName: data.selectedFeature?.product?.name ?? (isEdit ? initialSpec.productName : undefined),
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
