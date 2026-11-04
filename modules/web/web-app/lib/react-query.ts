import { Code, ConnectError } from '@connectrpc/connect'
import { QueryClient } from '@tanstack/react-query'

// Codes that a retry can never fix. Retrying them just adds latency: a 403 from
// RBAC would otherwise stall a page through 3 exponentially-backed-off retries.
const NON_RETRYABLE = new Set<Code>([
  Code.PermissionDenied,
  Code.Unauthenticated,
  Code.InvalidArgument,
  Code.NotFound,
  Code.Unimplemented,
  Code.AlreadyExists,
  Code.FailedPrecondition,
])

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 1000 * 60 * 5, // 5 minutes
      retry: (failureCount, error) =>
        !NON_RETRYABLE.has(ConnectError.from(error).code) && failureCount < 3,
    },
  },
})
