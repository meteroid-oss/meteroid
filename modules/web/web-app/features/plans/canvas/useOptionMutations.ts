import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'

import { usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { rpcErrorMessage } from '@/lib/rpcError'
import {
  createPlanOption,
  removePlanOption,
  reorderPlanOptions,
  updatePlanOption,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'

/** The writes of a draft version's options: what buyers pick between. */
export const useOptionMutations = () => {
  const queryClient = useQueryClient()
  const { version } = usePlanWithVersion()
  const planVersionId = version?.id ?? ''
  // The version carries its options, the components their links.
  const refresh = () =>
    Promise.all([
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: createPlanOption.parent, cardinality: undefined }),
      }),
      queryClient.invalidateQueries({
        queryKey: createConnectQueryKey({ schema: listPriceComponents, cardinality: undefined }),
      }),
    ])
  const failed = (fallback: string) => (error: unknown) =>
    toast.error(rpcErrorMessage(error, fallback))

  const add = useMutation(createPlanOption, {
    onSuccess: refresh,
    onError: failed('The option could not be added'),
  })
  const update = useMutation(updatePlanOption, {
    onSuccess: refresh,
    onError: failed('The option could not be saved'),
  })
  const remove = useMutation(removePlanOption, {
    onSuccess: refresh,
    onError: failed('The option could not be removed'),
  })
  const reorder = useMutation(reorderPlanOptions, {
    onSuccess: refresh,
    onError: failed('The options could not be reordered'),
  })

  return {
    pending: add.isPending || update.isPending || remove.isPending || reorder.isPending,
    /** A new option priced as the default one: its cells link the same prices. */
    add: (name: string) => add.mutateAsync({ planVersionId, name }),
    rename: (optionId: string, name: string, alias?: string) =>
      update.mutateAsync({ optionId, name, alias }),
    setHidden: (optionId: string, hidden: boolean) => update.mutateAsync({ optionId, hidden }),
    remove: (optionId: string) => remove.mutateAsync({ optionId }),
    reorder: (optionIds: string[]) => reorder.mutateAsync({ planVersionId, optionIds }),
  }
}

export type OptionMutations = ReturnType<typeof useOptionMutations>
