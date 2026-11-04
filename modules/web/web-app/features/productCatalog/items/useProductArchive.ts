import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { useCallback } from 'react'
import { toast } from 'sonner'

import {
  archiveProduct,
  unarchiveProduct,
} from '@/rpc/api/products/v1/products-ProductsService_connectquery'

export const useProductArchive = () => {
  const queryClient = useQueryClient()

  const invalidate = () =>
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({ schema: archiveProduct.parent, cardinality: undefined }),
    })

  const archiveMut = useMutation(archiveProduct, {
    onSuccess: async () => {
      await invalidate()
      toast.success('Product archived')
    },
    onError: e => toast.error(`Failed to archive product: ${e.message}`),
  })

  const unarchiveMut = useMutation(unarchiveProduct, {
    onSuccess: async () => {
      await invalidate()
      toast.success('Product unarchived')
    },
    onError: e => toast.error(`Failed to unarchive product: ${e.message}`),
  })

  const { mutate: archiveMutate } = archiveMut
  const { mutate: unarchiveMutate } = unarchiveMut

  return {
    archive: useCallback((productId: string) => archiveMutate({ productId }), [archiveMutate]),
    unarchive: useCallback(
      (productId: string) => unarchiveMutate({ productId }),
      [unarchiveMutate]
    ),
    isPending: archiveMut.isPending || unarchiveMut.isPending,
  }
}
