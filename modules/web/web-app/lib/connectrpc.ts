import {
  DisableQuery,
  MethodUnaryDescriptor,
  disableQuery,
  useQuery as useQueryUnsafe,
} from '@connectrpc/connect-query'
import { CreateQueryOptions } from '@connectrpc/connect-query/dist/cjs/create-use-query-options'
import { matchPath } from 'react-router-dom'
import { toast } from 'sonner'

import { getSessionToken } from '@/features/auth/session'

import type { Message, PartialMessage, PlainMessage } from '@bufbuild/protobuf'
import type { ConnectError, Interceptor, Transport } from '@connectrpc/connect'
import type { UseQueryResult } from '@tanstack/react-query'

const loggingInterceptorSkipError = ['AbortError:', 'DOMException:']
export const loggingInterceptor: Interceptor = next => async req => {
  try {
    const result = await next(req)
    console.log(`🔃 to ${req.method.name} `, req.message, result?.message)
    return result
  } catch (e) {
    const error = e
    const errorStr = String(e)

    // only error if it doesn't start with the strings in the array
    if (!loggingInterceptorSkipError.some(s => errorStr.startsWith(s))) {
      console.error(`🚨 to ${req.method.name} `, req.message, error)
    }

    throw error
  }
}

const errorInterceptorSkipError = ['TypeError:', 'AbortError:', 'DOMException:']
export const errorInterceptor: Interceptor = next => async req => {
  try {
    return await next(req)
  } catch (e) {
    const errorStr = String(e)

    if (!errorInterceptorSkipError.some(s => errorStr.startsWith(s))) {
      toast.error(errorStr)
    }
    throw e
  }
}

export const authInterceptor: Interceptor = next => async req => {
  const path = window.location.pathname
  const { tenantSlug } = matchPath('/tenant/:tenantSlug/*', path)?.params ?? {}

  const token = getSessionToken()

  tenantSlug && req.header.append('x-md-tenant-slug', tenantSlug)
  token && req.header.append('Authorization', `Bearer ${token}`)

  const result = await next(req)
  return result
}

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
