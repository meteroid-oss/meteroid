import { createQueryOptions, useTransport } from '@connectrpc/connect-query'
import { useQueries } from '@tanstack/react-query'

import { getFeature } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'

/** Full features by id, for references that only carry a name and code. */
export const useFeatures = (ids: string[]) => {
  const transport = useTransport()
  const results = useQueries({
    queries: ids.map(id => createQueryOptions(getFeature, { id }, { transport })),
  })
  return new Map(results.flatMap(r => (r.data?.feature ? [[r.data.feature.id, r.data.feature] as const] : [])))
}
