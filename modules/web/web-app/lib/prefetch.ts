import { createQueryOptions } from '@connectrpc/connect-query'

import { queryClient } from '@/lib/react-query'
import { getTransport } from '@/lib/transport'

import type { DescMessage, DescMethodUnary, MessageInitShape } from '@bufbuild/protobuf'

// Warm the react-query cache for a connect-query unary method from a router
// loader. `createQueryOptions` is the exact helper the `useQuery` hook uses
// internally, so the cache key matches and the component reads the in-flight
// query instead of starting a second fetch.
//
// Intentionally not awaited by callers: the loader kicks the request off at
// navigation/route-match time, then the route renders immediately while the
// data streams in. Returns the promise so a caller can await it when blocking
// navigation on the data is actually desired.
export const prefetchQuery = <I extends DescMessage, O extends DescMessage>(
  schema: DescMethodUnary<I, O>,
  input?: MessageInitShape<I>
) => queryClient.ensureQueryData(createQueryOptions(schema, input, { transport: getTransport() }))
