import { Navigate, RouteObject } from 'react-router-dom'

import { Catalog, CatalogOutlet } from '@/pages/tenants/catalog'
import { ProductItems } from '@/pages/tenants/catalog/productItems'
import { ProductMetrics } from '@/pages/tenants/catalog/productMetrics'

export const productCatalogRoutes: RouteObject = {
  path: 'metrics',
  children: [
    {
      index: true,
      element: <Catalog />,
    },
    {
      path: ':familyExternalId',
      element: <CatalogOutlet />,
      children: [
        {
          index: true,
          element: <Navigate to="items" />,
        },
        {
          path: 'items',
          element: <ProductItems />,
        },
        {
          path: 'metrics',
          element: <ProductMetrics />,
        },
      ],
    },
  ],
}
