import { skipToken } from '@connectrpc/connect-query'
import { ReactNode, createContext, useContext, useMemo } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { getBillableMetric } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import { BillableMetric } from '@/rpc/api/billablemetrics/v1/models_pb'
import { ListPriceComponentResponse } from '@/rpc/api/pricecomponents/v1/pricecomponents_pb'
import { Product } from '@/rpc/api/products/v1/models_pb'
import { getProduct } from '@/rpc/api/products/v1/products-ProductsService_connectquery'

interface ComponentRefs {
  products: Map<string, Product>
  metrics: Map<string, BillableMetric>
}

const ComponentRefsContext = createContext<ComponentRefs | null>(null)

/** Shares the products and metrics `ListPriceComponents` returns with the cards it renders. */
export const ComponentRefsProvider = ({
  list,
  children,
}: {
  list: ListPriceComponentResponse | undefined
  children: ReactNode
}) => {
  const value = useMemo(
    () => ({
      products: new Map(list?.products.map(p => [p.id, p])),
      metrics: new Map(list?.metrics.map(m => [m.id, m])),
    }),
    [list]
  )
  return <ComponentRefsContext.Provider value={value}>{children}</ComponentRefsContext.Provider>
}

/** The product from the surrounding list when it has it, else fetched on its own. */
export const useComponentProduct = (productId: string | undefined) => {
  const refs = useContext(ComponentRefsContext)
  const known = productId ? refs?.products.get(productId) : undefined
  const fetched = useQuery(getProduct, productId && !known ? { productId } : skipToken).data
  return known ?? fetched?.product
}

export const useComponentMetric = (metricId: string | undefined) => {
  const refs = useContext(ComponentRefsContext)
  const known = metricId ? refs?.metrics.get(metricId) : undefined
  const fetched = useQuery(getBillableMetric, metricId && !known ? { id: metricId } : skipToken).data
  return known ?? fetched?.billableMetric
}
