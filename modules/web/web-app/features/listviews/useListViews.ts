import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'

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
