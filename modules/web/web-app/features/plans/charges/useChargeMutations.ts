import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'

import { usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { existingPriceEntries, wrapAsNewPriceEntries } from '@/features/pricing/conversions'
import { rpcErrorMessage } from '@/lib/rpcError'
import { getResolvedEntitlementsForPlanVersion } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import {
  clearComponentPrice,
  createPriceComponent,
  editPriceComponent,
  listPriceComponents,
  removePriceComponent,
  setComponentPrice,
} from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'

import type {
  PriceComponent,
  PriceInput,
  ProductRef,
} from '@/rpc/api/pricecomponents/v1/models_pb'

/** The writes of the plan editor's pricing: one cell, or the charge's structure. */
export const useChargeMutations = () => {
  const queryClient = useQueryClient()
  const { version } = usePlanWithVersion()
  const refresh = () => {
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({ schema: listPriceComponents, cardinality: undefined }),
    })
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({
        schema: getResolvedEntitlementsForPlanVersion.parent,
        cardinality: undefined,
      }),
    })
  }
  const failed = (fallback: string) => (error: unknown) =>
    toast.error(rpcErrorMessage(error, fallback))

  const set = useMutation(setComponentPrice, {
    onSuccess: refresh,
    onError: failed('The price could not be saved'),
  })
  const clear = useMutation(clearComponentPrice, {
    onSuccess: refresh,
    onError: failed('The price could not be removed'),
  })
  const edit = useMutation(editPriceComponent, {
    onSuccess: refresh,
    onError: failed('The charge could not be saved'),
  })
  const add = useMutation(createPriceComponent, {
    onSuccess: refresh,
    onError: failed('The charge could not be added'),
  })
  const drop = useMutation(removePriceComponent, {
    onSuccess: refresh,
    onError: failed('The charge could not be removed'),
  })
  const planVersionId = version?.id ?? ''

  return {
    pending:
      set.isPending || clear.isPending || edit.isPending || add.isPending || drop.isPending,
    /** One price in the input's currency, for each listed option; none listed: every option. */
    setCell: (component: Pick<PriceComponent, 'id'>, optionIds: string[], price: PriceInput) =>
      set.mutateAsync({ priceComponentId: component.id, optionIds, price }),
    /**
     * The whole cell in the prices' currency, for each listed option (none listed: every
     * option): a capacity charge's levels, every level it held before replaced.
     */
    setCellPrices: (component: Pick<PriceComponent, 'id'>, optionIds: string[], prices: PriceInput[]) =>
      set.mutateAsync({ priceComponentId: component.id, optionIds, prices }),
    /** Empties those options' cells in `currency`; none listed: every option. */
    clearCell: (component: Pick<PriceComponent, 'id'>, optionIds: string[], currency: string) =>
      clear.mutateAsync({ priceComponentId: component.id, optionIds, currency }),
    /** A new name, every price kept. */
    rename: (component: PriceComponent, name: string) =>
      edit.mutateAsync({
        planVersionId,
        component: { id: component.id, name, productId: component.productId },
        prices: existingPriceEntries(component.prices.map(price => price.id)),
      }),
    /** A new kind: its prices are replaced by `prices` (none: included, no price). */
    restructure: (component: PriceComponent, name: string, prices: PriceInput[]) =>
      edit.mutateAsync({
        planVersionId,
        component: { id: component.id, name, productId: component.productId },
        prices: wrapAsNewPriceEntries(prices),
      }),
    remove: (component: Pick<PriceComponent, 'id'>) =>
      drop.mutateAsync({ priceComponentId: component.id }),
    create: (name: string, product: ProductRef, prices: PriceInput[]) =>
      add.mutateAsync({ planVersionId, name, product, prices: wrapAsNewPriceEntries(prices) }),
  }
}

export type ChargeMutations = ReturnType<typeof useChargeMutations>
