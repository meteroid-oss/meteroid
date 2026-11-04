import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query'
import { Badge, Button, Tooltip, TooltipContent, TooltipTrigger } from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { useAtom } from 'jotai'
import { PencilIcon, Trash2Icon } from 'lucide-react'
import { ReactNode } from 'react'
import { Link } from 'react-router-dom'
import { toast } from 'sonner'

import { metricUnitFromProto } from '@/features/billing/metrics/units'
import { useIsDraftVersion, usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { PriceComponentProperty } from '@/features/plans/pricecomponents/components/PriceComponentProperty'
import { PricingDetailsView } from '@/features/plans/pricecomponents/components/PricingDetailsView'
import {
  cadencesAdverb,
  cadencesPeriod,
  humanizeKey,
  isIncludedPack,
} from '@/features/plans/pricecomponents/components/pricingFormat'
import {
  editedComponentsAtom,
  useCurrency,
} from '@/features/plans/pricecomponents/utils'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { formatCadence } from '@/lib/mapping/prices'
import { rpcErrorMessage } from '@/lib/rpcError'
import { getBillableMetric } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import { getResolvedEntitlementsForPlanVersion } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import {
  listPriceComponents,
  removePriceComponent,
} from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import { FeeStructure_BillingType } from '@/rpc/api/prices/v1/models_pb'
import { getProduct } from '@/rpc/api/products/v1/products-ProductsService_connectquery'
import { useConfirmationModal } from 'providers/ConfirmationProvider'

import type { BillableMetric } from '@/rpc/api/billablemetrics/v1/models_pb'
import type { PriceComponent as ProtoPriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { FeeStructure, Price } from '@/rpc/api/prices/v1/models_pb'

// --- Main card ---

export const PriceComponentCard: React.FC<{
  component: ProtoPriceComponent
}> = ({ component }) => {
  const planWithVersion = usePlanWithVersion()
  const queryClient = useQueryClient()
  const [, setEditedComponents] = useAtom(editedComponentsAtom)
  const isDraft = useIsDraftVersion()
  const currency = useCurrency()

  const product = useQuery(
    getProduct,
    component.productId ? { productId: component.productId } : skipToken
  ).data?.product
  const structure = product?.feeStructure?.structure
  const metricId =
    structure?.case === 'usage' || structure?.case === 'capacity'
      ? structure.value.metricId
      : undefined
  const metric = useQuery(getBillableMetric, metricId ? { id: metricId } : skipToken).data
    ?.billableMetric

  const deleteComponentMutation = useMutation(removePriceComponent, {
    onSuccess: () => {
      planWithVersion.version &&
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({
            schema: listPriceComponents,
            cardinality: undefined
          })
        })
      queryClient.invalidateQueries({ queryKey: createConnectQueryKey({
        schema: getResolvedEntitlementsForPlanVersion.parent,
        cardinality: undefined
      }) })
    },
    onError: error => toast.error(rpcErrorMessage(error, 'The price component could not be removed')),
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
    <article
      className="flex flex-col gap-4 px-4 py-4 border border-border shadow-sm rounded-lg max-w-4xl group bg-card"
      key={component.id}
    >
      <header className="flex flex-wrap items-start justify-between gap-x-4 gap-y-1 min-h-8">
        <div className="flex items-center gap-2 min-w-0">
          <h4 className="text-base font-semibold text-foreground truncate">{component.name}</h4>
          <Badge variant="secondary" size="sm" className="shrink-0">
            {componentKindLabel(component.prices)}
          </Badge>
          {product && product.name !== component.name && (
            <Badge size="sm" className="shrink-0 border-transparent bg-brand/10 text-brand shadow-none hover:bg-brand/10">
              {product.name}
            </Badge>
          )}
        </div>
        <div className="flex items-center gap-2">
          <span className="text-sm text-muted-foreground">
            {billingSummary(component.prices, structure, metric)}
          </span>
          {isDraft && (
            <div className="flex opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    className="!rounded-r-none bg-transparent text-destructive hover:text-destructive"
                    onClick={removeComponent}
                    size="icon"
                    aria-label="Delete price component"
                  >
                    <Trash2Icon size={12} strokeWidth={2} />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>Delete price component</TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    className="!rounded-l-none"
                    onClick={editComponent}
                    size="icon"
                    aria-label="Edit price component"
                  >
                    <PencilIcon size={12} strokeWidth={2} />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>Edit price component</TooltipContent>
              </Tooltip>
            </div>
          )}
        </div>
      </header>
      <PricingDetailsView
        prices={component.prices}
        currency={currency}
        usageUnit={metricUnitFromProto(metric)}
        slotUnitName={structure?.case === 'slot' ? structure.value.unitName : undefined}
      />
    </article>
  )
}

const componentKindLabel = (prices: Price[]): string => {
  const price = prices[0]
  const inCredits = price?.creditUnit ? ' · in credits' : ''
  switch (price?.pricing.case) {
    case 'creditPricing':
      return 'Credit'
    case 'usagePricing':
      return `Usage${inCredits}`
    case 'capacityPricing':
      return `Commitment${inCredits}`
    case 'ratePricing':
      return 'Flat rate'
    case 'slotPricing':
      return 'Seats'
    case 'extraRecurringPricing':
      return 'Recurring fee'
    case 'oneTimePricing':
      return 'One-time fee'
    default:
      return 'Price'
  }
}

const billingSummary = (
  prices: Price[],
  structure: FeeStructure['structure'] | undefined,
  metric: BillableMetric | undefined
): ReactNode => {
  const pricing = prices[0]?.pricing
  if (!pricing) return null
  const billed = `Billed ${cadencesAdverb(prices.map(p => p.cadence))}`
  const metricLink = metric ? <MetricLink metric={metric} /> : null

  switch (pricing.case) {
    case 'creditPricing':
      if (isIncludedPack(pricing.value)) {
        return pricing.value.once
          ? 'Included once, at the start'
          : `Included each ${cadencesPeriod(prices.map(p => p.cadence))}`
      }
      return pricing.value.once ? 'Billed once, on the first invoice' : `${billed}, in advance`
    case 'ratePricing':
    case 'slotPricing':
      return `${billed}, in advance`
    case 'capacityPricing':
      return metricLink ? (
        <>
          {billed}, in advance · overage on {metricLink}
        </>
      ) : (
        `${billed}, in advance`
      )
    case 'usagePricing': {
      const model = pricing.value.model
      const dimensions =
        model.case === 'matrix' && model.value.rows[0]
          ? [model.value.rows[0].dimension1?.key, model.value.rows[0].dimension2?.key]
              .filter((key): key is string => Boolean(key))
              .map(key => humanizeKey(key).toLowerCase())
          : []
      const pricedBy = dimensions.length ? `, priced by ${dimensions.join(' and ')}` : ''
      return metricLink ? (
        <>
          {billed} on {metricLink}
          {pricedBy}
        </>
      ) : (
        `${billed}${pricedBy}`
      )
    }
    case 'extraRecurringPricing': {
      const billingType =
        structure?.case === 'extraRecurring' ? structure.value.billingType : undefined
      if (billingType === FeeStructure_BillingType.ADVANCE) return `${billed}, in advance`
      if (billingType === FeeStructure_BillingType.ARREAR) return `${billed}, in arrears`
      return billed
    }
    case 'oneTimePricing':
      return 'Billed once'
    default:
      return null
  }
}

const MetricLink = ({ metric }: { metric: BillableMetric }) => {
  const basePath = useBasePath()
  return (
    <Link to={`${basePath}/metrics/${metric.id}`} className="text-brand hover:underline">
      {metric.name}
    </Link>
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
    case 'creditPricing': {
      const packs = pricing.value.quantity
      return {
        pricingModel: 'Credit pack',
        cadence: `${pricing.value.once ? 'Once at start' : cadenceStr}${packs > 1 ? ` (x${packs})` : ''}`,
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

const DisplayBillableMetric = ({ metricId }: { metricId: string }) => {
  const metric = useQuery(getBillableMetric, { id: metricId })
  return metric.isLoading ? <></> : <>{metric.data?.billableMetric?.name}</>
}
