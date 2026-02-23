import {
  createConnectQueryKey,
  createProtobufSafeUpdater,
  useMutation,
} from '@connectrpc/connect-query'
import { Button } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useAtom } from 'jotai'
import { ChevronDownIcon, ChevronRightIcon, PencilIcon, Trash2Icon } from 'lucide-react'
import { useMemo, useState } from 'react'

import { LocalId } from '@/components/LocalId'
import { useIsDraftVersion, usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { PriceComponentProperty } from '@/features/plans/pricecomponents/components/PriceComponentProperty'
import { PricingDetailsView } from '@/features/plans/pricecomponents/components/PricingDetailsView'
import {
  editedComponentsAtom,
  useCurrency,
} from '@/features/plans/pricecomponents/utils'
import { useQuery } from '@/lib/connectrpc'
import { formatCadence } from '@/lib/mapping/prices'
import { getBillableMetric } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import {
  listPriceComponents,
  removePriceComponent,
} from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import { getProduct } from '@/rpc/api/products/v1/products-ProductsService_connectquery'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

import type { PriceComponent as ProtoPriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'

// --- Main card ---

export const PriceComponentCard: React.FC<{
  component: ProtoPriceComponent
}> = ({ component }) => {
  const [isCollapsed, setIsCollapsed] = useState(true)
  const planWithVersion = usePlanWithVersion()
  const queryClient = useQueryClient()
  const [, setEditedComponents] = useAtom(editedComponentsAtom)
  const isDraft = useIsDraftVersion()
  const currency = useCurrency()

  const summary = useMemo(() => deriveSummary(component.prices), [component.prices])

  const deleteComponentMutation = useMutation(removePriceComponent, {
    onSuccess: () => {
      planWithVersion.version &&
        queryClient.setQueryData(
          createConnectQueryKey(listPriceComponents, {
            planVersionId: planWithVersion.version.id,
          }),
          createProtobufSafeUpdater(listPriceComponents, prev => ({
            components: prev?.components.filter(c => c.id !== component.id) ?? [],
          }))
        )
    },
  })

  const showConfirmationModal = useConfirmationModal()

  const removeComponent = async () => {
    showConfirmationModal(() =>
      deleteComponentMutation.mutate({ priceComponentId: component.id })
    )
  }

  const editComponent = () => {
    setEditedComponents(prev => [...prev, component.id])
  }

  return (
    <div
      className="flex flex-col grow px-4 py-4 border border-border shadow-sm rounded-lg max-w-4xl group bg-card"
      key={component.id}
    >
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
            <h4 className="text-base text-accent-1 font-semibold">{component.name}</h4>
            {component.productId && <DisplayProductBadge productId={component.productId} />}
            <LocalId localId={component.localId} className="max-w-24" />
          </div>
        </div>
        {isDraft && (
          <div className="align-end hidden group-hover:flex">
            <Button
              variant="ghost"
              className=" !rounded-r-none bg-transparent text-destructive hover:text-destructive"
              onClick={removeComponent}
              size="icon"
            >
              <Trash2Icon size={12} strokeWidth={2} />
            </Button>
            <Button
              variant="ghost"
              className=" !rounded-l-none"
              onClick={editComponent}
              size="icon"
            >
              <PencilIcon size={12} strokeWidth={2} />
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
            {component.productId && (
              <ProductLinkedItem productId={component.productId} />
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
            <PricingDetailsView prices={component.prices} currency={currency} />
          </div>
        )}
      </div>
    </div>
  )
}

// --- Summary derivation ---

export interface PriceSummary {
  pricingModel: string
  cadence: string
}

export function deriveSummary(prices: Price[]): PriceSummary {
  if (prices.length === 0) return { pricingModel: '-', cadence: '-' }

  const pricing = prices[0].pricing
  const cadences = [...new Set(prices.map(p => formatCadence(p.cadence)))]
  const cadenceStr = cadences.join(' or ')

  switch (pricing.case) {
    case 'ratePricing':
      return { pricingModel: 'Rate', cadence: cadenceStr }
    case 'slotPricing':
      return { pricingModel: 'Slot-based', cadence: cadenceStr }
    case 'capacityPricing':
      return { pricingModel: 'Committed capacity', cadence: cadenceStr }
    case 'usagePricing': {
      const model = pricing.value.model
      const modelName =
        model.case === 'perUnit'
          ? 'Per Unit'
          : model.case === 'tiered'
            ? 'Tiered'
            : model.case === 'volume'
              ? 'Volume'
              : model.case === 'package'
                ? 'Package'
                : model.case === 'matrix'
                  ? 'Matrix'
                  : 'Usage'
      return { pricingModel: modelName, cadence: cadenceStr }
    }
    case 'extraRecurringPricing': {
      const qty = pricing.value.quantity
      return {
        pricingModel: 'Fixed fee',
        cadence: `${cadenceStr}${qty > 1 ? ` (x${qty})` : ''}`,
      }
    }
    case 'oneTimePricing': {
      const qty = pricing.value.quantity
      return {
        pricingModel: 'Fixed fee',
        cadence: `One-time${qty > 1 ? ` (x${qty})` : ''}`,
      }
    }
    default:
      return { pricingModel: '-', cadence: '-' }
  }
}

// --- Product-linked structural info (metric, unit name) ---

export const ProductLinkedItem = ({ productId }: { productId: string }) => {
  const product = useQuery(getProduct, { productId })
  if (!product.data?.product?.feeStructure) return null

  const structure = product.data.product.feeStructure.structure
  switch (structure.case) {
    case 'slot':
      return (
        <PriceComponentProperty
          label="Unit type"
          className="col-span-1 border-r border-border pr-4"
          childrenClassNames="truncate"
        >
          {structure.value.unitName}
        </PriceComponentProperty>
      )
    case 'capacity':
      return (
        <PriceComponentProperty
          label="Billable Metric"
          className="col-span-1 border-r border-border pr-4"
          childrenClassNames="truncate"
        >
          <DisplayBillableMetric metricId={structure.value.metricId} />
        </PriceComponentProperty>
      )
    case 'usage':
      return (
        <PriceComponentProperty
          label="Billable Metric"
          className="col-span-1 border-r border-border pr-4"
          childrenClassNames="truncate"
        >
          <DisplayBillableMetric metricId={structure.value.metricId} />
        </PriceComponentProperty>
      )
    default:
      return null
  }
}

// --- Shared display components ---

const DisplayProductBadge = ({ productId }: { productId: string }) => {
  const product = useQuery(getProduct, { productId })
  if (product.isLoading || !product.data?.product) return null
  return (
    <span className="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium bg-brand/10 text-brand">
      {product.data.product.name}
    </span>
  )
}

const DisplayBillableMetric = ({ metricId }: { metricId: string }) => {
  const metric = useQuery(getBillableMetric, { id: metricId })
  return metric.isLoading ? <></> : <>{metric.data?.billableMetric?.name}</>
}
