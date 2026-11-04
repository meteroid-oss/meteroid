import { createConnectQueryKey } from '@connectrpc/connect-query'

import {
  getCreditUnit,
  listCreditUnitPrices,
  listCreditUnits,
} from '@/rpc/api/creditunits/v1/creditunits-CreditUnitsService_connectquery'

import type { DescMethodUnary } from '@bufbuild/protobuf'
import type { QueryClient } from '@tanstack/react-query'

const CREDIT_UNIT_QUERIES: DescMethodUnary[] = [
  listCreditUnits,
  getCreditUnit,
  listCreditUnitPrices,
]

export const invalidateCreditUnits = (queryClient: QueryClient) =>
  Promise.all(
    CREDIT_UNIT_QUERIES.map(schema =>
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema, cardinality: undefined }),
      })
    )
  )
