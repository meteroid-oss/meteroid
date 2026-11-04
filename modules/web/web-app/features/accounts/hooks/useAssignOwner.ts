import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'

import {
  assignCustomerOwner,
  getCustomerById,
  listCustomers,
} from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { listSignals } from '@/rpc/api/signals/v1/signals-SignalsService_connectquery'

import type { DescMethodUnary } from '@bufbuild/protobuf'
import type { ConnectError } from '@connectrpc/connect'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
const OWNER_QUERIES: DescMethodUnary<any, any>[] = [listSignals, listCustomers, getCustomerById]

export const useAssignOwner = () => {
  const queryClient = useQueryClient()
  return useMutation(assignCustomerOwner, {
    onSuccess: () =>
      Promise.all(
        OWNER_QUERIES.map(schema =>
          queryClient.invalidateQueries({
            queryKey: createConnectQueryKey({ schema, cardinality: undefined }),
          })
        )
      ),
    onError: (error: ConnectError) => toast.error(error.rawMessage || 'Failed to assign the owner'),
  })
}
