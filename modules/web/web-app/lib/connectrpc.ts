// Re-exported from connect-query. In connect-query v2, `useQuery` already
// accepts `skipToken` for disabling and infers the required input fields from
// the method's request schema, so no custom wrapper is needed.
export { useMutation, useQuery } from '@connectrpc/connect-query'
