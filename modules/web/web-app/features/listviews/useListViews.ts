import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'

import { usePrefetch } from '@/hooks/usePrefetch'
import { useQuery } from '@/lib/connectrpc'
import {
  createView,
  deleteView,
  duplicateView,
  listViews,
  setDefaultView,
  updateView,
} from '@/rpc/api/listviews/v1/listviews-ListViewsService_connectquery'
import { EntityListType } from '@/rpc/api/listviews/v1/models_pb'

/** Saved views for one list (shared + the caller's private ones), with their mutations. */
export function useListViews(listType: EntityListType) {
  const queryClient = useQueryClient()

  const query = useQuery(listViews, { entityType: listType })

  const invalidate = () =>
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({ schema: listViews, cardinality: 'finite' }),
    })

  const opts = { onSuccess: () => invalidate() }

  return {
    query,
    views: query.data?.views ?? [],
    defaultViewId: query.data?.defaultViewId,
    create: useMutation(createView, opts),
    update: useMutation(updateView, opts),
    remove: useMutation(deleteView, opts),
    duplicate: useMutation(duplicateView, opts),
    setDefault: useMutation(setDefaultView, opts),
  }
}

const LIST_TYPES = [
  EntityListType.CUSTOMER,
  EntityListType.SUBSCRIPTION,
  EntityListType.INVOICE,
  EntityListType.CREDIT_NOTE,
  EntityListType.QUOTE,
]

/**
 * A list waits for its saved views before its first query (to open the default view), so loading
 * them up front, once the app is idle, turns that wait into a cache hit.
 */
export function usePrefetchListViews(tenantSlug?: string) {
  const prefetch = usePrefetch()

  useEffect(() => {
    const run = () => LIST_TYPES.forEach(entityType => prefetch(listViews, { entityType }))
    if (typeof window.requestIdleCallback !== 'function') {
      const id = setTimeout(run, 1000)
      return () => clearTimeout(id)
    }
    const id = window.requestIdleCallback(run, { timeout: 2000 })
    return () => window.cancelIdleCallback(id)
  }, [prefetch, tenantSlug])
}
