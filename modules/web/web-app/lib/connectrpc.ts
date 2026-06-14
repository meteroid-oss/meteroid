import { useMutation, useQuery as useQueryUnsafe } from '@connectrpc/connect-query'

import type {
  DescMessage,
  DescMethodUnary,
  MessageInitShape,
  MessageShape,
} from '@bufbuild/protobuf'
import type { ConnectError } from '@connectrpc/connect'
import type { SkipToken, UseQueryOptions } from '@connectrpc/connect-query'
import type { UseQueryResult } from '@tanstack/react-query'

// Field names of a message, excluding the protobuf-es brand keys ($typeName/$unknown).
type FieldKeys<I extends DescMessage> = Exclude<keyof MessageShape<I>, '$typeName' | '$unknown'>
type HasFields<I extends DescMessage> = [FieldKeys<I>] extends [never] ? false : true

// A version of useQuery that forces passing the input message when the request
// has any fields, while keeping it optional for empty requests.
export function useQuery<
  I extends DescMessage,
  O extends DescMessage,
  SelectOutData = MessageShape<O>,
>(
  schema: DescMethodUnary<I, O>,
  ...args: HasFields<I> extends true
    ? [input: SkipToken | MessageInitShape<I>, options?: UseQueryOptions<O, SelectOutData>]
    : [input?: SkipToken | MessageInitShape<I>, options?: UseQueryOptions<O, SelectOutData>]
): UseQueryResult<SelectOutData, ConnectError> {
  const [input, queryOptions] = args
  return useQueryUnsafe(schema, input, queryOptions)
}

export { useMutation }
