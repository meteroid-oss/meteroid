import { Code, ConnectError } from '@connectrpc/connect'
import { QueryClient } from '@tanstack/react-query'

// Connect codes that reflect a terminal/logical outcome rather than a transient
// failure — retrying them only delays the UI (a completed checkout session
// returns FailedPrecondition; the default 3 retries left the page blank for
// seconds before it could redirect).
const NON_RETRYABLE_CODES = new Set<Code>([
  Code.InvalidArgument,
  Code.NotFound,
  Code.AlreadyExists,
  Code.PermissionDenied,
  Code.Unauthenticated,
  Code.FailedPrecondition,
  Code.Unimplemented,
])

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 1000 * 60 * 5, // 5 minutes
      retry: (failureCount, error) => {
        if (error instanceof ConnectError && NON_RETRYABLE_CODES.has(error.code)) {
          return false
        }
        return failureCount < 3
      },
    },
  },
})
