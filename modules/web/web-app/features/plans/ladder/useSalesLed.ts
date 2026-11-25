import {
  createConnectQueryKey,
  createQueryOptions,
  useMutation,
  useTransport,
} from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { useCallback } from 'react'
import { toast } from 'sonner'

import { rpcErrorMessage } from '@/lib/rpcError'
import { PlanOverview } from '@/rpc/api/plans/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import {
  getPlanPricingPage,
  getPricingPage,
  updatePlanPricingPage,
} from '@/rpc/api/pricingpage/v1/pricingpage-PricingPageService_connectquery'

/** Sells a plan of the ladder by contacting sales, or self-serve again; its copy stays. */
export const useToggleSalesLed = () => {
  const transport = useTransport()
  const queryClient = useQueryClient()
  const { mutateAsync } = useMutation(updatePlanPricingPage)

  return useCallback(
    async (plan: PlanOverview) => {
      const salesLed = !plan.salesLed
      try {
        // The update replaces every field: read them fresh, not from the cache.
        const { page } = await queryClient.fetchQuery({
          ...createQueryOptions(getPlanPricingPage, { planId: plan.id }, { transport }),
          staleTime: 0,
        })
        await mutateAsync({
          page: {
            tagline: page?.tagline,
            popular: page?.popular,
            ctaLabel: page?.ctaLabel,
            highlights: page?.highlights,
            planId: plan.id,
            salesLed,
            hidePrice: !!page?.hidePrice,
            contactUrl: page?.contactUrl,
            productOrder: page?.productOrder,
            hiddenProductIds: page?.hiddenProductIds,
          },
        })
        await Promise.all([
          queryClient.invalidateQueries({
            queryKey: createConnectQueryKey({ schema: listPlans.parent, cardinality: undefined }),
          }),
          queryClient.invalidateQueries({
            queryKey: createConnectQueryKey({
              schema: getPricingPage.parent,
              cardinality: undefined,
            }),
          }),
        ])
        toast.success(
          salesLed ? `${plan.name} is sold by contacting sales` : `${plan.name} is sold self-serve`
        )
      } catch (error) {
        toast.error(rpcErrorMessage(error, 'The plan could not be changed'))
      }
    },
    [transport, queryClient, mutateAsync]
  )
}
