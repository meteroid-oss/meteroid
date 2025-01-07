import {
  DisableQuery,
  MethodUnaryDescriptor,
  disableQuery,
  useQuery as useQueryUnsafe,
} from '@connectrpc/connect-query'
import { CreateQueryOptions } from '@connectrpc/connect-query/dist/cjs/create-use-query-options'

import type { Message, PartialMessage, PlainMessage } from '@bufbuild/protobuf'
import type { ConnectError, Transport } from '@connectrpc/connect'
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
