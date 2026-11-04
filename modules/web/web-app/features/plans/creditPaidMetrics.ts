import { createQueryOptions, useTransport } from '@connectrpc/connect-query'
import { useQueries } from '@tanstack/react-query'

import { useCreditUnits } from '@/features/pricing/CreditUnitPriceFields'
import { useQuery } from '@/lib/connectrpc'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import { getProduct } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

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
 * by metric id. `products` holds each component's product, when loaded.
 */
export const creditPaidLabels = (
  components: Pick<PriceComponent, 'productId' | 'prices'>[],
  products: Map<string, Pick<Product, 'feeStructure'>>,
  units: Pick<CreditUnit, 'id' | 'pluralName'>[]
): Map<string, string> => {
  const labels = new Map<string, string>()
  for (const component of components) {
    const unitId = creditUnitOf(component)
    const metricId = metricOf(component.productId ? products.get(component.productId) : undefined)
    if (!unitId || !metricId) continue
    const unit = units.find(u => u.id === unitId)?.pluralName ?? 'credits'
    labels.set(metricId, `Paid with credit (${unit})`)
  }
  return labels
}

/** The labels of the metrics `planVersionId` prices in a credit unit. */
export const useCreditPaidLabels = (planVersionId: string) => {
  const transport = useTransport()
  const components =
    useQuery(listPriceComponents, { planVersionId }).data?.components.filter(
      c => creditUnitOf(c) && c.productId
    ) ?? []
  const products = useQueries({
    queries: components.map(c =>
      createQueryOptions(getProduct, { productId: c.productId! }, { transport })
    ),
  })
  const units = useCreditUnits() ?? []
  const loaded = new Map(
    products.flatMap(result => {
      const product = result.data?.product
      return product ? [[product.id, product] as const] : []
    })
  )
  return creditPaidLabels(components, loaded, units)
}
