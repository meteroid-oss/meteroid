import { create, type MessageInitShape } from '@bufbuild/protobuf'
import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Skeleton,
  cn,
} from '@md/ui'
import { keepPreviousData, useQueryClient } from '@tanstack/react-query'
import { MoreHorizontal, Plus } from 'lucide-react'
import { useState } from 'react'

import { Gate } from '@/components/Gate'
import { EntityEntitlementDialog } from '@/features/entitlements/EntityEntitlementDialog'
import { FeatureCreateSheet } from '@/features/entitlements/features/FeatureCreateSheet'
import { entitlementValueLabel, meteredValueToWrite } from '@/features/entitlements/utils'
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
} from '@/rpc/api/entitlements/v1/models_pb'

/** "Included", "Off", "10,000 per billing period": a default value as the plan page reads one. */
const valueText = (value: Entitlement['value'] | undefined): string => {
  const v = value?.value
  if (v?.case === 'booleanValue') return v.value.enabled ? 'Included' : 'Off'
  if (v?.case === 'meteredValue' && !v.value.enabled) return `${entitlementValueLabel(v)} · off`
  return entitlementValueLabel(v)
}

const ProductFeatureRow = ({ feature }: { feature: Feature }) => {
  const queryClient = useQueryClient()
  const [editOpen, setEditOpen] = useState(false)

  const featureEntity = {
    EntityId: { case: 'featureId' as const, value: feature.id },
  } as EntitlementEntity

  const entitlementQuery = useQuery(listEntitlementsByEntity, { entity: featureEntity })
  const existing: Entitlement | undefined = entitlementQuery.data?.entitlements?.[0]
  const isMetered = feature.featureType?.Inner?.case === 'metered'
  const isBoolean = feature.featureType?.Inner?.case === 'boolean'

  const invalidate = () =>
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({
        schema: listEntitlementsByEntity.parent,
        cardinality: undefined,
      }),
    })

  const updateMutation = useMutation(updateEntitlement, { onSuccess: invalidate })

  const handleToggleDisable = () => {
    if (!existing) return
    const v = existing.value?.value
    if (!v) return
    let flipped: MessageInitShape<typeof EntitlementValueSchema>
    if (v.case === 'booleanValue') {
      flipped = { value: { case: 'booleanValue' as const, value: { enabled: !v.value.enabled } } }
    } else if (v.case === 'meteredValue') {
      flipped = meteredValueToWrite(v.value, !v.value.enabled)
    } else {
      return
    }
    updateMutation.mutate({ id: existing.id, value: create(EntitlementValueSchema, flipped) })
  }

  const currentValue = existing?.value?.value
  const isDisabled =
    (currentValue?.case === 'booleanValue' && !currentValue.value.enabled) ||
    (currentValue?.case === 'meteredValue' && !currentValue.value.enabled)
  const canToggle = !!existing && (isBoolean || isMetered)

  return (
    <>
      <div
        className={cn(
          'grid grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)_2rem] items-center gap-x-4 px-4 py-2.5 text-sm',
          isDisabled && 'opacity-60'
        )}
      >
        <span className="min-w-0 truncate font-medium">{feature.name}</span>
        <span className="min-w-0 truncate">
          {entitlementQuery.isLoading ? (
            <Skeleton className="h-4 w-20" />
          ) : existing ? (
            <span className="font-semibold tabular-nums">{valueText(existing.value)}</span>
          ) : (
            <span className="text-muted-foreground">No default</span>
          )}
        </span>
        <span className="flex justify-end">
          <Gate perm="catalog:manage">
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-7"
                  disabled={updateMutation.isPending}
                  aria-label={`${feature.name} actions`}
                >
                  <MoreHorizontal size={14} />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onSelect={() => setEditOpen(true)}>
                  {existing ? 'Change the default value' : 'Set a default value'}
                </DropdownMenuItem>
                {canToggle && (
                  <DropdownMenuItem onSelect={handleToggleDisable}>
                    {isDisabled ? 'Turn on by default' : 'Turn off by default'}
                  </DropdownMenuItem>
                )}
              </DropdownMenuContent>
            </DropdownMenu>
          </Gate>
        </span>
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
  )
}

const PAGE_SIZE = 100

/** The product's features with the value each grants by default, as the plan page lists them. */
export const ProductEntitlementsSection = ({ productId }: { productId: string }) => {
  const queryClient = useQueryClient()
  const [addOpen, setAddOpen] = useState(false)
  const [shown, setShown] = useState(PAGE_SIZE)

  const featuresQuery = useQuery(
    listFeatures,
    {
      productId,
      pagination: { page: 0, perPage: shown },
      statuses: [FeatureStatus.ACTIVE, FeatureStatus.DISABLED],
    },
    { placeholderData: keepPreviousData }
  )
  const features = featuresQuery.data?.features ?? []
  const total = featuresQuery.data?.paginationMeta?.totalItems ?? 0

  const invalidateFeatures = () =>
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({
        schema: listFeatures.parent,
        cardinality: undefined,
      }),
    })

  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-center justify-between gap-3">
        <div className="flex flex-wrap items-baseline gap-x-2">
          <h3 className="text-sm font-semibold">Entitlements</h3>
          <span className="text-xs text-muted-foreground">
            What every plan selling it grants, unless the plan sets its own
          </span>
        </div>
        <Gate perm="catalog:manage">
          <Button
            type="button"
            size="sm"
            variant="outline"
            hasIcon
            onClick={() => setAddOpen(true)}
          >
            <Plus size={12} /> Entitlement
          </Button>
        </Gate>
      </div>

      {featuresQuery.isLoading ? (
        <div className="flex flex-col gap-2">
          <Skeleton className="h-8 w-full" />
          <Skeleton className="h-8 w-full" />
        </div>
      ) : features.length === 0 ? (
        <p className="text-sm text-muted-foreground">No features linked to this product.</p>
      ) : (
        <div className="divide-y divide-border overflow-hidden rounded-lg border border-border">
          {features.map(f => (
            <ProductFeatureRow key={f.id} feature={f} />
          ))}
        </div>
      )}
      {total > features.length && (
        <div className="flex justify-center">
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={featuresQuery.isFetching}
            onClick={() => setShown(n => n + PAGE_SIZE)}
          >
            Show more ({total - features.length} left)
          </Button>
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
    </section>
  )
}
