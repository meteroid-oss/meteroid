import {
  DisableQuery,
  MethodUnaryDescriptor,
  disableQuery,
  useMutation,
  useQuery as useQueryUnsafe
} from '@connectrpc/connect-query'

import type { Message, PartialMessage, PlainMessage } from '@bufbuild/protobuf'
import type { ConnectError, Transport } from '@connectrpc/connect'
// CreateQueryOptions is re-exported from the package root as UseQueryOptions; under
// moduleResolution "bundler" the previous deep import into dist/cjs is no longer resolvable.
import type { UseQueryOptions as CreateQueryOptions } from '@connectrpc/connect-query'
import type { UseQueryResult } from '@tanstack/react-query'

type HasFields<T> = keyof T extends never ? false : true

// // a version of useQuery that forces to use all the required fields of the input message, if any
export function useQuery<I extends Message<I>, O extends Message<O>, SelectOutData = O>(
  methodSig: MethodUnaryDescriptor<I, O>,
  ...args: HasFields<PlainMessage<I>> extends true
    ? [
        input: DisableQuery | PlainMessage<I>,
        options?: Omit<CreateQueryOptions<I, O, SelectOutData>, 'transport'> & {
          transport?: Transport
        },
      ]
    : [
        input?: DisableQuery | undefined,
        options?: Omit<CreateQueryOptions<I, O, SelectOutData>, 'transport'> & {
          transport?: Transport
        },
      ]
): UseQueryResult<SelectOutData, ConnectError> {
  const [input, queryOptions] = args
  return useQueryUnsafe(
    methodSig,
    input as PartialMessage<I> | typeof disableQuery | undefined,
    queryOptions
  )
}

export { useMutation }
