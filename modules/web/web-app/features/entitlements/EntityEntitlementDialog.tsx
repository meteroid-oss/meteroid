/**
 * EntityEntitlementDialog — EntitlementDialog wrapper for existing entities (plans, subscriptions, etc.).
 * Makes live API calls: createEntitlement, updateEntitlement, or createFeature+entitlement atomically.
 * Use for adding/editing entitlements on entities that already exist in the backend.
 */
import { PartialMessage } from '@bufbuild/protobuf'
import { useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'

import {
  EntitlementDialog,
  EntitlementFormValues,
} from '@/features/entitlements/EntitlementDialog'
import { useQuery } from '@/lib/connectrpc'
import {
  createEntitlement,
  createFeature,
  listEntitlementsByEntity,
  listFeatures,
  updateEntitlement,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import {
  CalendarUnit,
  Entitlement,
  EntitlementEntity,
  EntitlementValue,
  OverageBehavior,
} from '@/rpc/api/entitlements/v1/models_pb'

interface Props {
  entity: PartialMessage<EntitlementEntity>
  existing?: Entitlement
  onClose: () => void
  featureId?: string
  featureIsMetered?: boolean
  existingFeatureIds?: Set<string>
  /**
   * Pre-fill the form with this value without treating it as an edit (no `existing` row).
   * Used when overriding an inherited entitlement — submitting creates a new local row with
   * the (possibly modified) seeded values. When provided, the dialog renders as "Override".
   */
  seedValue?: EntitlementValue
}

function valueToFormValues(value: EntitlementValue | undefined): Partial<EntitlementFormValues> {
  const v = value?.value
  const isBoolean = v?.case !== 'meteredValue'
  const metered = v?.case === 'meteredValue' ? v.value : undefined
  const boolVal = v?.case === 'booleanValue' ? v.value : undefined

  const rp = metered?.resetPeriod?.Inner
  const resetPeriodType =
    !rp ? 'never'
    : rp.case === 'billingCycle' ? 'billingCycle'
    : rp.case === 'never' ? 'never'
    : rp.case === 'calendar' ? 'calendar'
    : rp.case === 'fixedWindow' ? 'fixedWindow'
    : rp.case === 'slidingWindow' ? 'slidingWindow'
    : 'never'

  const hasInterval = rp?.case === 'calendar' || rp?.case === 'fixedWindow' || rp?.case === 'slidingWindow'

  const overageBehaviorInner = metered?.overageBehavior?.Inner
  const overageBehaviorType =
    !overageBehaviorInner ? 'none'
    : overageBehaviorInner.case === 'allow' ? 'allow'
    : overageBehaviorInner.case === 'block' ? 'block'
    : 'none'
  const gracePeriodPct =
    overageBehaviorInner?.case === 'block' ? overageBehaviorInner.value.gracePeriodPct : undefined

  return {
    featureType: isBoolean ? 'boolean' : 'metered',
    boolEnabled: boolVal ? boolVal.enabled : true,
    limit: metered?.limit ?? '',
    resetPeriodType: resetPeriodType as EntitlementFormValues['resetPeriodType'],
    resetUnit: hasInterval
      ? (rp as { case: string; value: { unit: CalendarUnit } }).value.unit
      : CalendarUnit.MONTH,
    resetInterval: hasInterval
      ? (rp as { case: string; value: { interval: number } }).value.interval
      : 1,
    overageBehaviorType: overageBehaviorType as EntitlementFormValues['overageBehaviorType'],
    gracePeriodPct,
    warningThresholdPct: metered?.warningThresholdPct,
    meteredEnabled: metered?.enabled ?? true,
  }
}

function buildOverageBehavior(
  overageBehaviorType: EntitlementFormValues['overageBehaviorType'],
  gracePeriodPct: number | undefined
): OverageBehavior | undefined {
  if (overageBehaviorType === 'allow') {
    return new OverageBehavior({ Inner: { case: 'allow', value: {} } })
  }
  if (overageBehaviorType === 'block') {
    return new OverageBehavior({ Inner: { case: 'block', value: { gracePeriodPct } } })
  }
  return undefined
}

function buildValue(
  isBoolean: boolean,
  data: Pick<
    EntitlementFormValues,
    | 'boolEnabled'
    | 'limit'
    | 'resetPeriodType'
    | 'resetUnit'
    | 'resetInterval'
    | 'overageBehaviorType'
    | 'gracePeriodPct'
    | 'warningThresholdPct'
    | 'meteredEnabled'
  >
) {
  if (isBoolean) {
    return {
      value: {
        case: 'booleanValue' as const,
        value: { enabled: data.boolEnabled ?? true },
      },
    }
  }
  const resetPeriod =
    data.resetPeriodType === 'billingCycle'
      ? { Inner: { case: 'billingCycle' as const, value: {} } }
      : data.resetPeriodType === 'calendar'
        ? { Inner: { case: 'calendar' as const, value: { unit: data.resetUnit!, interval: data.resetInterval! } } }
        : data.resetPeriodType === 'fixedWindow'
          ? { Inner: { case: 'fixedWindow' as const, value: { unit: data.resetUnit!, interval: data.resetInterval! } } }
          : data.resetPeriodType === 'slidingWindow'
            ? { Inner: { case: 'slidingWindow' as const, value: { unit: data.resetUnit!, interval: data.resetInterval! } } }
            : { Inner: { case: 'never' as const, value: {} } }

  return {
    value: {
      case: 'meteredValue' as const,
      value: {
        limit: data.limit || undefined,
        resetPeriod,
        overageBehavior: buildOverageBehavior(data.overageBehaviorType, data.gracePeriodPct),
        warningThresholdPct: data.warningThresholdPct,
        enabled: data.meteredEnabled ?? true,
      },
    },
  }
}

export const EntityEntitlementDialog = ({
  entity,
  existing,
  onClose,
  featureId,
  featureIsMetered,
  existingFeatureIds,
  seedValue,
}: Props) => {
  const queryClient = useQueryClient()
  const isEdit = !!existing
  const isOverride = !isEdit && !!seedValue

  const featuresQuery = useQuery(listFeatures, { pagination: { page: 0, perPage: 100 }, statuses: [] })
  const featureMap = Object.fromEntries(
    (featuresQuery.data?.features ?? []).map(f => [f.id, f])
  )

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: [listEntitlementsByEntity.service.typeName] })

  const createEntitlementMutation = useMutation(createEntitlement, {
    onSuccess: () => { invalidate(); onClose() },
    onError: err => toast.error(`Failed to create entitlement: ${err.message}`),
  })
  const updateMutation = useMutation(updateEntitlement, {
    onSuccess: () => { invalidate(); onClose() },
    onError: err => toast.error(`Failed to update entitlement: ${err.message}`),
  })
  const createFeatureMutation = useMutation(createFeature, {
    onSuccess: () => { invalidate(); onClose() },
    onError: err => toast.error(`Failed to create feature: ${err.message}`),
  })

  const isPending =
    createEntitlementMutation.isPending ||
    updateMutation.isPending ||
    createFeatureMutation.isPending

  const resolvedFeatureId = featureId ?? existing?.featureId
  const lockedFeature = resolvedFeatureId
    ? {
        id: resolvedFeatureId,
        name: featureMap[resolvedFeatureId]?.name ?? resolvedFeatureId,
        isMetered: featureId != null
          ? (featureIsMetered ?? false)
          : existing?.value?.value?.case === 'meteredValue',
      }
    : undefined

  const initialValues = existing
    ? valueToFormValues(existing.value)
    : seedValue
      ? valueToFormValues(seedValue)
      : undefined

  const handleSubmit = async (data: EntitlementFormValues) => {
    const effectiveFeatureId = data.featureId ?? featureId
    const value = new EntitlementValue(buildValue(data.featureType === 'boolean', data))

    if (isEdit) {
      updateMutation.mutate({
        id: existing.id,
        value,
      })
    } else if (effectiveFeatureId) {
      createEntitlementMutation.mutate({
        featureId: effectiveFeatureId,
        entity: entity as EntitlementEntity,
        value,
      })
    } else {
      createFeatureMutation.mutate({
        name: data.featureName!,
        description: data.featureDescription || undefined,
        featureType:
          data.featureType === 'boolean'
            ? { Inner: { case: 'boolean', value: {} } }
            : { Inner: { case: 'metered', value: { metricId: data.metricId! } } },
        entitlement: {
          entity: entity as EntitlementEntity,
          value,
        },
      })
    }
  }

  return (
    <EntitlementDialog
      open
      onOpenChange={onClose}
      onSubmit={handleSubmit}
      initialValues={initialValues}
      lockedFeature={lockedFeature}
      existingFeatureIds={existingFeatureIds}
      title={isEdit ? 'Edit Entitlement' : isOverride ? 'Override Entitlement' : 'Add Entitlement'}
      submitLabel={isEdit ? 'Save' : isOverride ? 'Save override' : 'Add'}
      isSubmitting={isPending}
    />
  )
}
