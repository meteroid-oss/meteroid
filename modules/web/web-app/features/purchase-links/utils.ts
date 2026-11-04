import { createConnectQueryKey } from '@connectrpc/connect-query'
import { QueryClient } from '@tanstack/react-query'

import { listPurchaseLinks } from '@/rpc/api/purchaselinks/v1/purchaselinks-PurchaseLinksService_connectquery'

export const invalidatePurchaseLinks = (queryClient: QueryClient) =>
  queryClient.invalidateQueries({
    queryKey: createConnectQueryKey({ schema: listPurchaseLinks.parent, cardinality: undefined }),
  })
