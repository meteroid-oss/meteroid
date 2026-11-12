import { createQueryOptions, useTransport } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { useCallback } from 'react'

import type { DescMessage, DescMethodUnary, MessageInitShape } from '@bufbuild/protobuf'

/**
 * Warms the cache for a query a page will run on mount, under the same key `useQuery` uses.
 * Fresh entries are left alone (default staleTime), so calling it on every hover is cheap.
 */
export const usePrefetch = () => {
  const queryClient = useQueryClient()
  const transport = useTransport()

  return useCallback(
    <I extends DescMessage, O extends DescMessage>(
      schema: DescMethodUnary<I, O>,
      input: MessageInitShape<I>
    ) => {
      void queryClient.prefetchQuery(createQueryOptions(schema, input, { transport }))
    },
    [queryClient, transport]
  )
}
