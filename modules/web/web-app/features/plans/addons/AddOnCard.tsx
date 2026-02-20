import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import { Button } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { Trash2Icon } from 'lucide-react'

import { LocalId } from '@/components/LocalId'
import { useIsDraftVersion, usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { PriceComponentProperty } from '@/features/plans/pricecomponents/components/PriceComponentProperty'
import {
  feeTypeIcon,
  feeTypeToHuman,
  priceSummaryBadges,
  useCurrency,
} from '@/features/plans/pricecomponents/utils'
import { formatCadence } from '@/lib/mapping/prices'
import {
  detachAddOnFromPlanVersion,
  listAddOns,
} from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { FeeType } from '@/rpc/api/prices/v1/models_pb'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

import type { AddOn } from '@/rpc/api/addons/v1/models_pb'
import type { ComponentFeeType } from '@/features/pricing/conversions'

export function feeTypeEnumToComponentFeeType(feeType: FeeType): ComponentFeeType {
  switch (feeType) {
    case FeeType.RATE:
      return 'rate'
    case FeeType.SLOT:
      return 'slot'
    case FeeType.CAPACITY:
      return 'capacity'
    case FeeType.USAGE:
      return 'usage'
    case FeeType.EXTRA_RECURRING:
      return 'extraRecurring'
    case FeeType.ONE_TIME:
      return 'oneTime'
  }
}

interface Props {
  addOn: AddOn
}

export const AddOnCard: React.FC<Props> = ({ addOn }) => {
  const planWithVersion = usePlanWithVersion()
  const queryClient = useQueryClient()
  const isDraft = useIsDraftVersion()
  const currency = useCurrency()
  const showConfirmationModal = useConfirmationModal()

  const componentFeeType = feeTypeEnumToComponentFeeType(addOn.feeType)
  const Icon = feeTypeIcon(componentFeeType)
  const feeLabel = feeTypeToHuman(componentFeeType)
  const priceBadges = priceSummaryBadges(componentFeeType, addOn.price, currency)
  const cadence = addOn.price ? formatCadence(addOn.price.cadence) : '-'

  const detachMutation = useMutation(detachAddOnFromPlanVersion, {
    onSuccess: () => {
      if (planWithVersion.version) {
        queryClient.setQueryData(
          createConnectQueryKey(listAddOns, {
            planVersionId: planWithVersion.version.id,
          }),
          createProtobufSafeUpdater(listAddOns, prev => ({
            addOns: prev?.addOns.filter(a => a.id !== addOn.id) ?? [],
          }))
        )
      }
    },
  })

  const handleRemove = () => {
    if (!planWithVersion.version) return
    showConfirmationModal(() =>
      detachMutation.mutate({
        planVersionId: planWithVersion.version!.id,
        addOnId: addOn.id,
      })
    )
  }

  return (
    <div className="flex flex-col grow px-4 py-4 border border-border shadow-sm rounded-lg max-w-4xl group bg-card">
      <div className="flex flex-row items-center min-h-9">
        <div className="flex items-center gap-2 flex-grow">
          <Icon className="w-4 h-4 text-muted-foreground" />
          <h4 className="text-base text-accent-1 font-semibold">{addOn.name}</h4>
          <span className="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium bg-muted text-muted-foreground">
            {feeLabel}
          </span>
          <LocalId localId={addOn.localId} className="max-w-24" />
        </div>
        {isDraft && (
          <div className="hidden group-hover:flex">
            <Button
              variant="ghost"
              className="bg-transparent text-destructive hover:text-destructive"
              onClick={handleRemove}
              size="icon"
            >
              <Trash2Icon size={12} strokeWidth={2} />
            </Button>
          </div>
        )}
      </div>
      <div className="flex flex-col grow px-6 mt-2">
        <div className="grid grid-cols-3 gap-x-6">
          <PriceComponentProperty
            label="Pricing model"
            className="col-span-1 border-r border-border pr-4"
          >
            <span>{feeLabel}</span>
          </PriceComponentProperty>
          <PriceComponentProperty
            label="Price"
            className="col-span-1 border-r border-border pr-4"
          >
            <span>{priceBadges.join(' / ')}</span>
          </PriceComponentProperty>
          <PriceComponentProperty label="Cadence" className="col-span-1">
            <span>{cadence}</span>
          </PriceComponentProperty>
        </div>
      </div>
    </div>
  )
}
