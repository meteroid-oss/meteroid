import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import { Button } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ChevronDownIcon, ChevronRightIcon, Trash2Icon } from 'lucide-react'
import { useMemo, useState } from 'react'

import { LocalId } from '@/components/LocalId'
import { useIsDraftVersion, usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import {
  deriveSummary,
  ProductLinkedItem,
} from '@/features/plans/pricecomponents/PriceComponentCard'
import { PriceComponentProperty } from '@/features/plans/pricecomponents/components/PriceComponentProperty'
import { PricingDetailsView } from '@/features/plans/pricecomponents/components/PricingDetailsView'
import { useCurrency } from '@/features/plans/pricecomponents/utils'
import {
  detachAddOnFromPlanVersion,
  listAddOns,
} from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { FeeType } from '@/rpc/api/prices/v1/models_pb'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

import type { ComponentFeeType } from '@/features/pricing/conversions'
import type { AddOn } from '@/rpc/api/addons/v1/models_pb'

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
  const [isCollapsed, setIsCollapsed] = useState(true)
  const planWithVersion = usePlanWithVersion()
  const queryClient = useQueryClient()
  const isDraft = useIsDraftVersion()
  const planCurrency = useCurrency()
  const currency = addOn.price?.currency ?? planCurrency
  const showConfirmationModal = useConfirmationModal()

  const prices = addOn.price ? [addOn.price] : []
  const summary = useMemo(() => deriveSummary(prices), [addOn.price])

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
      <div
        className="mt-0.5 flex flex-row min-h-9"
        onClick={() => setIsCollapsed(!isCollapsed)}
      >
        <div className="flex flex-row items-center cursor-pointer w-full">
          <div className="mr-2">
            {isCollapsed ? (
              <ChevronRightIcon className="w-5 l-5 text-accent-1 group-hover:text-muted-foreground" />
            ) : (
              <ChevronDownIcon className="w-5 l-5 text-accent-1 group-hover:text-muted-foreground" />
            )}
          </div>
          <div className="flex items-center gap-2">
            <h4 className="text-base text-accent-1 font-semibold">{addOn.name}</h4>
            <span className="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium bg-muted text-muted-foreground">
              Add-on
            </span>
            <LocalId localId={addOn.id} className="max-w-24" />
          </div>
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
      <div className="flex flex-col grow px-7">
        <div className="flex flex-col">
          <div className="grid grid-cols-3 gap-x-6 mt-4">
            <PriceComponentProperty
              label="Pricing model"
              className="col-span-1 border-r border-border pr-4"
            >
              <span>{summary.pricingModel}</span>
            </PriceComponentProperty>
            {addOn.productId && (
              <ProductLinkedItem productId={addOn.productId} />
            )}
            <PriceComponentProperty
              label="Cadence"
              className="col-span-1 border-r border-border last:border-none pr-4"
            >
              <span>{summary.cadence}</span>
            </PriceComponentProperty>
          </div>
        </div>
        {!isCollapsed && (
          <div className="mt-6 flex flex-col grow">
            <PricingDetailsView prices={prices} currency={currency} />
          </div>
        )}
      </div>
    </div>
  )
}
