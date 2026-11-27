import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { Badge, cn } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { ChevronDownIcon, ChevronRightIcon, Trash2Icon } from 'lucide-react'
import { useMemo, useState } from 'react'

import { LocalId } from '@/components/LocalId'
import { RowActionButton } from '@/components/RowActionButton'
import { containedTable } from '@/features/customers/cards/ContainedTableSection'
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
    case FeeType.CREDIT:
      return 'credit'
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
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({
          schema: listAddOns.parent,
          cardinality: undefined,
        }),
      })
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
    // A framed item as the price components: its name on top (a click shows its prices), what it
    // is below; in a draft, Remove on the right, always visible.
    <div className={cn('overflow-hidden rounded-lg border border-border bg-card', containedTable)}>
      <div className="flex items-center justify-between gap-4 px-4 py-3">
        <button
          type="button"
          className="flex min-w-0 flex-1 items-center gap-2 text-left"
          onClick={() => setIsCollapsed(!isCollapsed)}
          aria-expanded={!isCollapsed}
        >
          {isCollapsed ? (
            <ChevronRightIcon className="size-4 shrink-0 text-muted-foreground" />
          ) : (
            <ChevronDownIcon className="size-4 shrink-0 text-muted-foreground" />
          )}
          <h4 className="truncate text-[15px] font-medium text-foreground">{addOn.name}</h4>
          <Badge variant="secondary" size="sm" className="shrink-0">
            Add-on
          </Badge>
        </button>
        <div className="flex shrink-0 items-center gap-1.5">
          <LocalId
            localId={addOn.id}
            className="max-w-24"
            buttonClassName="h-6 px-2 py-0 text-[11px]"
          />
          {isDraft && (
            <RowActionButton destructive onClick={handleRemove} aria-label="Remove add-on">
              <Trash2Icon />
            </RowActionButton>
          )}
        </div>
      </div>
      <div className="flex flex-col border-t border-border px-4 py-3">
        <div className="flex flex-col">
          <div className="grid grid-cols-3 gap-x-6">
            <PriceComponentProperty
              label="Pricing model"
              className="col-span-1 border-r border-border pr-4"
            >
              <span>{summary.pricingModel}</span>
            </PriceComponentProperty>
            {addOn.productId && <ProductLinkedItem productId={addOn.productId} />}
            <PriceComponentProperty
              label="Cadence"
              className="col-span-1 border-r border-border last:border-none pr-4"
            >
              <span>{summary.cadence}</span>
            </PriceComponentProperty>
          </div>
        </div>
        {!isCollapsed && (
          <div className="mt-4 flex grow flex-col border-t border-border pt-4">
            <PricingDetailsView prices={prices} currency={currency} />
          </div>
        )}
      </div>
    </div>
  )
}
