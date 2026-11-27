import { skipToken } from '@connectrpc/connect-query'
import { keepPreviousData } from '@tanstack/react-query'
import { useRef } from 'react'

import { useDebounceValue } from '@/hooks/useDebounce'
import { useQuery } from '@/lib/connectrpc'
import { previewQuoteTotals } from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'

import type { CreateQuote } from '@/rpc/api/quotes/v1/models_pb'

const DEBOUNCE_MS = 400

const bigintSafe = (_: string, v: unknown) => (typeof v === 'bigint' ? v.toString() : v)

/**
 * Server-computed totals of the unsaved editor state. `quote` is undefined while the editor
 * cannot send it yet; the last totals stay shown while the next ones are requested.
 */
export function useQuoteTotals(quote: CreateQuote | undefined, quoteId?: string) {
  const key = quote ? `${quoteId ?? ''}:${JSON.stringify(quote, bigintSafe)}` : ''
  const latest = useRef<{ key: string; quote?: CreateQuote }>({ key: '' })
  latest.current = { key, quote }

  const debouncedKey = useDebounceValue(key, DEBOUNCE_MS)
  const settled = debouncedKey !== '' && debouncedKey === latest.current.key

  const query = useQuery(
    previewQuoteTotals,
    settled && latest.current.quote ? { quote: latest.current.quote, quoteId } : skipToken,
    {
      queryKeyHashFn: () => `previewQuoteTotals-${debouncedKey}`,
      placeholderData: keepPreviousData,
      staleTime: 60_000,
      retry: false,
    }
  )

  return {
    totals: debouncedKey ? query.data?.totals : undefined,
    /** The draft's saved phases, priced as edited. */
    priceSchedule: debouncedKey ? query.data?.priceSchedule : undefined,
    /** Nothing to show yet: the first totals of this state are on their way. */
    isLoading: debouncedKey !== '' && !query.data && !query.isError,
    isUpdating: key !== debouncedKey || query.isFetching,
    isError: query.isError,
  }
}
