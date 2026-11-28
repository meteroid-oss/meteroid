import { useComponentMetric, useComponentProduct } from '@/features/plans/pricecomponents/componentRefs'
import { PriceComponentProperty } from '@/features/plans/pricecomponents/components/PriceComponentProperty'
import { formatCadence } from '@/lib/mapping/prices'

import type { Price } from '@/rpc/api/prices/v1/models_pb'

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
                : model.case === 'dimensionRates' || model.case === 'rateCard'
                  ? 'By dimension'
                  : model.case === 'percentage'
                    ? 'Percentage'
                    : model.case === 'dynamic'
                      ? 'Price from events'
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

export const ProductLinkedItem = ({ productId }: { productId: string }) => {
  const product = useComponentProduct(productId)
  if (!product?.feeStructure) return null

  const structure = product.feeStructure.structure
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

const DisplayBillableMetric = ({ metricId }: { metricId: string }) => {
  const metric = useComponentMetric(metricId)
  return <>{metric?.name}</>
}
