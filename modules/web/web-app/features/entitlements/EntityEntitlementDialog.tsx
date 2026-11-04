import { create, type MessageInitShape } from '@bufbuild/protobuf';
import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query';
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'

import {
  EntitlementDialog,
  EntitlementFormValues,
} from '@/features/entitlements/EntitlementDialog'
import {
  buildConfigValue,
  buildMeteredPolicy,
  configFeatureTypeValue,
  meteredPolicyToFields,
  parseConfigMap,
} from '@/features/entitlements/creation/types'
import { featureKindFromProto, slugifyCode } from '@/features/entitlements/utils'
import { useQuery } from '@/lib/connectrpc'
import {
  createEntitlement,
  createFeature,
  getFeature,
  listEntitlementsByEntity,
  updateEntitlement,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import {
  CalendarUnit,
  Entitlement,
  EntitlementEntity,
  EntitlementEntitySchema,
  EntitlementValueSchema,
  ResetPeriodSchema,
} from '@/rpc/api/entitlements/v1/models_pb';

import type { EntitlementValue } from '@/rpc/api/entitlements/v1/models_pb';

interface Props {
  entity: MessageInitShape<typeof EntitlementEntitySchema>
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
  const metered = v?.case === 'meteredValue' ? v.value : undefined
  const boolVal = v?.case === 'booleanValue' ? v.value : undefined
  const config = v?.case === 'configValue' ? v.value : undefined

  const featureType: EntitlementFormValues['featureType'] =
    v?.case === 'meteredValue' ? 'metered' : v?.case === 'configValue' ? 'config' : 'boolean'

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

  const cv = config?.value
  // A `json` value that is a flat object round-trips through the friendly map editor.
  const mapEntries = cv?.case === 'json' ? parseConfigMap(cv.value) : null

  return {
    featureType,
    boolEnabled: boolVal ? boolVal.enabled : true,
    limit: metered?.limit ?? '',
    resetPeriodType: resetPeriodType as EntitlementFormValues['resetPeriodType'],
    resetUnit: hasInterval
      ? (rp as { case: string; value: { unit: CalendarUnit } }).value.unit
      : CalendarUnit.MONTH,
    resetInterval: hasInterval
      ? (rp as { case: string; value: { interval: number } }).value.interval
      : 1,
    meteredEnabled: metered?.enabled ?? true,
    groupBy: metered?.groupBy ?? '',
    ...(metered ? meteredPolicyToFields(metered) : {}),
    configKind: cv?.case === 'json' ? (mapEntries ? 'map' : 'json') : (cv?.case ?? 'number'),
    configNumber: cv?.case === 'number' ? cv.value : undefined,
    configBoolean: cv?.case === 'boolean' ? cv.value : undefined,
    configText: cv?.case === 'text' ? cv.value : undefined,
    // Populate both editors' state; the feature's declared type picks which one renders.
    configJson: cv?.case === 'json' ? cv.value : undefined,
    configEntries: mapEntries ?? undefined,
  }
}

function buildValue(
  data: Pick<
    EntitlementFormValues,
    | 'featureType'
    | 'boolEnabled'
    | 'limit'
    | 'resetPeriodType'
    | 'resetUnit'
    | 'resetInterval'
    | 'meteredEnabled'
    | 'groupBy'
    | 'overageType'
    | 'gracePct'
    | 'thresholds'
    | 'configKind'
    | 'configNumber'
    | 'configBoolean'
    | 'configText'
    | 'configJson'
    | 'configEntries'
  >
): MessageInitShape<typeof EntitlementValueSchema> {
  if (data.featureType === 'boolean') {
    return {
      value: {
        case: 'booleanValue' as const,
        value: { enabled: data.boolEnabled ?? true },
      },
    }
  }
  if (data.featureType === 'config') {
    return {
      value: {
        case: 'configValue' as const,
        value: buildConfigValue(data),
      },
    }
  }
  const resetPeriod: MessageInitShape<typeof ResetPeriodSchema> =
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
        enabled: data.meteredEnabled ?? true,
        groupBy: data.groupBy?.trim() || undefined,
        ...buildMeteredPolicy(data),
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

  const resolvedFeatureId = featureId ?? existing?.featureId
  const featureQuery = useQuery(getFeature, resolvedFeatureId ? { id: resolvedFeatureId } : skipToken)
  const feature = featureQuery.data?.feature
  const featureMap = feature ? { [feature.id]: feature } : {}

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
      schema: listEntitlementsByEntity.parent,
      cardinality: undefined
    }) })

  const createEntitlementMutation = useMutation(createEntitlement, {
    onSuccess: () => { invalidate(); onClose() },
    onError: err => toast.error(`Failed to create entitlement: ${err.message}`),
  })
  const updateMutation = useMutation(updateEntitlement, {
    onSuccess: () => { invalidate(); onClose() },
    onError: err => toast.error(`Failed to update entitlement: ${err.message}`),
  })
  const createFeatureMutation = useMutation(createFeature, {
    onError: err => toast.error(`Failed to create feature: ${err.message}`),
  })

  const isPending =
    createEntitlementMutation.isPending ||
    updateMutation.isPending ||
    createFeatureMutation.isPending

  const lockedFeatureType = ((): 'boolean' | 'metered' | 'config' => {
    if (!resolvedFeatureId) return 'boolean'
    const ft = featureMap[resolvedFeatureId]?.featureType
    if (ft) return featureKindFromProto(ft).type
    if (featureId != null && featureIsMetered) return 'metered'
    const c = existing?.value?.value?.case ?? seedValue?.value?.case
    return c === 'meteredValue' ? 'metered' : c === 'configValue' ? 'config' : 'boolean'
  })()
  const lockedFeature = resolvedFeatureId
    ? {
        id: resolvedFeatureId,
        name: featureMap[resolvedFeatureId]?.name ?? resolvedFeatureId,
        featureType: lockedFeatureType,
      }
    : undefined

  const featureKind = resolvedFeatureId
    ? featureKindFromProto(featureMap[resolvedFeatureId]?.featureType)
    : undefined
  const featureConfigKind = featureKind?.type === 'config' ? featureKind.valueType : undefined
  const featureOptions = featureKind?.type === 'config' ? featureKind.options : undefined

  const baseInitialValues = existing
    ? valueToFormValues(existing.value)
    : seedValue
      ? valueToFormValues(seedValue)
      : undefined
  // The config value type is fixed by the feature — seed it (and select options) so the value
  // editor matches, overriding whatever kind was inferred from the stored value.
  const initialValues = featureConfigKind
    ? {
        ...(baseInitialValues ?? {}),
        configKind: featureConfigKind,
        ...(featureOptions ? { configOptions: featureOptions } : {}),
      }
    : baseInitialValues

  const handleSubmit = async (data: EntitlementFormValues) => {
    const effectiveFeatureId = data.featureId ?? featureId
    const value = create(EntitlementValueSchema, buildValue(data))

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
      const created = await createFeatureMutation.mutateAsync({
        name: data.featureName!,
        code: data.featureCode || slugifyCode(data.featureName!),
        description: data.featureDescription || undefined,
        featureType:
          data.featureType === 'boolean'
            ? { Inner: { case: 'boolean', value: {} } }
            : data.featureType === 'config'
              ? {
                  Inner: {
                    case: 'config',
                    value: configFeatureTypeValue(
                      data.configKind ?? 'number',
                      data.configOptions ?? []
                    ),
                  },
                }
              : { Inner: { case: 'metered', value: { metricId: data.metricId! } } },
      })
      createEntitlementMutation.mutate({
        featureId: created.feature!.id,
        entity: entity as EntitlementEntity,
        value,
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
