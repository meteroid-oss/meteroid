import { create, type MessageInitShape } from '@bufbuild/protobuf';
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query';
import {
  Button,
  Skeleton,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { CirclePower, Pencil, Plus } from 'lucide-react'
import { useState } from 'react'

import { EntityEntitlementDialog } from '@/features/entitlements/EntityEntitlementDialog'
import { entitlementTooltip } from '@/features/entitlements/entitlementTooltips'
import { FeatureCreateSheet } from '@/features/entitlements/features/FeatureCreateSheet'
import { entitlementValueLabel } from '@/features/entitlements/utils'
import { useQuery } from '@/lib/connectrpc'
import {
  listEntitlementsByEntity,
  listFeatures,
  updateEntitlement,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import {
  Entitlement,
  EntitlementEntity,
  EntitlementValueSchema,
  Feature,
  FeatureStatus,
} from '@/rpc/api/entitlements/v1/models_pb';


interface ProductFeatureRowProps {
  feature: Feature
}

const ProductFeatureRow = ({ feature }: ProductFeatureRowProps) => {
  const queryClient = useQueryClient()
  const [editOpen, setEditOpen] = useState(false)

  const featureEntity = { EntityId: { case: 'featureId' as const, value: feature.id } } as EntitlementEntity

  const entitlementQuery = useQuery(listEntitlementsByEntity, { entity: featureEntity })
  const existing: Entitlement | undefined = entitlementQuery.data?.entitlements?.[0]
  const isMetered = feature.featureType?.Inner?.case === 'metered'
  const isBoolean = feature.featureType?.Inner?.case === 'boolean'

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
      schema: listEntitlementsByEntity.parent,
      cardinality: undefined
    }) })

  const updateMutation = useMutation(updateEntitlement, { onSuccess: invalidate })

  const handleToggleDisable = () => {
    if (!existing) return
    const v = existing.value?.value
    if (!v) return
    let flipped: MessageInitShape<typeof EntitlementValueSchema>
    if (v.case === 'booleanValue') {
      flipped = { value: { case: 'booleanValue' as const, value: { enabled: !v.value.enabled } } }
    } else if (v.case === 'meteredValue') {
      const m = v.value
      flipped = { value: { case: 'meteredValue' as const, value: { limit: m.limit, resetPeriod: m.resetPeriod, enabled: !m.enabled } } }
    } else {
      return
    }
    updateMutation.mutate({ id: existing.id, value: create(EntitlementValueSchema, flipped) })
  }

  const currentValue = existing?.value?.value
  const isDisabled =
    (currentValue?.case === 'booleanValue' && !currentValue.value.enabled) ||
    (currentValue?.case === 'meteredValue' && !currentValue.value.enabled)

  return (
    <TooltipProvider>
      <>
        <div className="flex items-center justify-between px-3 py-2 text-sm">
          <span className="font-medium">{feature.name}</span>
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            {entitlementQuery.isLoading ? (
              <span>...</span>
            ) : (
              <span>{existing ? entitlementValueLabel(existing.value?.value) : 'No default'}</span>
            )}
            <Tooltip>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  className="p-1 hover:bg-muted rounded"
                  onClick={() => setEditOpen(true)}
                >
                  <Pencil size={12} />
                </button>
              </TooltipTrigger>
              <TooltipContent>{entitlementTooltip('product', 'override')}</TooltipContent>
            </Tooltip>
            {existing && (isBoolean || isMetered) && (
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    type="button"
                    className="p-1 hover:bg-muted rounded"
                    onClick={handleToggleDisable}
                    disabled={updateMutation.isPending}
                  >
                    {isDisabled
                      ? <CirclePower size={12} className="text-destructive" />
                      : <CirclePower size={12} className="text-primary" />}
                  </button>
                </TooltipTrigger>
                <TooltipContent>{entitlementTooltip('product', isDisabled ? 'enable' : 'disable')}</TooltipContent>
              </Tooltip>
            )}
          </div>
        </div>

        {editOpen && (
          <EntityEntitlementDialog
            entity={featureEntity}
            existing={existing}
            featureId={feature.id}
            featureIsMetered={isMetered}
            onClose={() => setEditOpen(false)}
          />
        )}
      </>
    </TooltipProvider>
  )
}

interface Props {
  productId: string
}

export const ProductEntitlementsSection = ({ productId }: Props) => {
  const queryClient = useQueryClient()
  const [addOpen, setAddOpen] = useState(false)

  const featuresQuery = useQuery(listFeatures, {
    productId,
    pagination: { page: 0, perPage: 100 },
    statuses: [FeatureStatus.ACTIVE],
  })
  const features = featuresQuery.data?.features ?? []

  const invalidateFeatures = () =>
    queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
      schema: listFeatures.parent,
      cardinality: undefined
    }) })

  return (
    <div>
      <div className="flex justify-end mb-3">
        <Button
          type="button"
          size="sm"
          variant="secondary"
          hasIcon
          onClick={() => setAddOpen(true)}
        >
          <Plus size={12} /> Add
        </Button>
      </div>

      {featuresQuery.isLoading ? (
        <div className="flex flex-col gap-2">
          <Skeleton className="h-8 w-full" />
          <Skeleton className="h-8 w-full" />
        </div>
      ) : features.length === 0 ? (
        <p className="text-sm text-muted-foreground">No features linked to this product.</p>
      ) : (
        <div className="border rounded-md divide-y">
          {features.map(f => (
            <ProductFeatureRow key={f.id} feature={f} />
          ))}
        </div>
      )}

      {addOpen && (
        <FeatureCreateSheet
          initialProductId={productId}
          onClose={() => {
            setAddOpen(false)
            invalidateFeatures()
          }}
        />
      )}
    </div>
  )
}
