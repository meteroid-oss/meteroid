import { useCreditUnits } from '@/features/pricing/CreditUnitPriceFields'
import { useQuery } from '@/lib/connectrpc'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'

import type { CreditUnit } from '@/rpc/api/creditunits/v1/models_pb'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { Product } from '@/rpc/api/products/v1/models_pb'

const creditUnitOf = (component: Pick<PriceComponent, 'prices'>) =>
  component.prices.find(price => price.creditUnit)?.creditUnit?.creditUnitId

const metricOf = (product?: Pick<Product, 'feeStructure'>) => {
  const structure = product?.feeStructure?.structure
  return structure?.case === 'usage' || structure?.case === 'capacity'
    ? structure.value.metricId
    : undefined
}

/**
 * "Paid with credit (Halyard credits)" for each metric the components price in a credit unit,
 * by metric id: a charge's metric is its own structure's; `products` holds a product's for a
 * charge sent without one.
 */
export const creditPaidLabels = (
  components: Pick<PriceComponent, 'productId' | 'prices' | 'feeStructure'>[],
  products: Map<string, Pick<Product, 'feeStructure'>>,
  units: Pick<CreditUnit, 'id' | 'pluralName'>[]
): Map<string, string> => {
  const labels = new Map<string, string>()
  for (const component of components) {
    const unitId = creditUnitOf(component)
    const metricId = metricOf(
      component.feeStructure
        ? component
        : component.productId
          ? products.get(component.productId)
          : undefined
    )
    if (!unitId || !metricId) continue
    const unit = units.find(u => u.id === unitId)?.pluralName ?? 'credits'
    labels.set(metricId, `Paid with credit (${unit})`)
  }
  return labels
}

/** The labels of the metrics `planVersionId` prices in a credit unit. */
export const useCreditPaidLabels = (planVersionId: string) => {
  const components =
    useQuery(listPriceComponents, { planVersionId }).data?.components.filter(creditUnitOf) ?? []
  const units = useCreditUnits() ?? []
  return creditPaidLabels(components, new Map(), units)
}
