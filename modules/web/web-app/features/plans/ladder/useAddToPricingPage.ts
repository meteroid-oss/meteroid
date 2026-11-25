import { ConnectError } from '@connectrpc/connect'
import {
  createConnectQueryKey,
  createQueryOptions,
  useMutation,
  useTransport,
} from '@connectrpc/connect-query'
import { QueryClient, useQueryClient } from '@tanstack/react-query'
import { useCallback } from 'react'
import { toast } from 'sonner'

import { ladderOf } from '@/features/plans/ladder/ladderOrder'
import { PlanOverview } from '@/rpc/api/plans/v1/models_pb'
import {
  listPlans,
  setSelfServeLadder,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { getPricingPage } from '@/rpc/api/pricingpage/v1/pricingpage-PricingPageService_connectquery'

// A ladder is saved whole: the product line's every plan is read first, not the page shown.
export const LADDER_PAGE_SIZE = 1000

/** Refetches what a ladder shows: the plans' ranks and the pricing page's cards. */
export const invalidateLadder = (queryClient: QueryClient) =>
  Promise.all([
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({ schema: listPlans.parent, cardinality: undefined }),
    }),
    queryClient.invalidateQueries({
      queryKey: createConnectQueryKey({ schema: getPricingPage, cardinality: undefined }),
    }),
  ])

type Edit = (ladder: string[], plan: PlanOverview) => string[]

const last: Edit = (ladder, plan) => [...ladder.filter(id => id !== plan.id), plan.id]
const without: Edit = (ladder, plan) => ladder.filter(id => id !== plan.id)

/** Changes a plan's place on its product line's pricing page, from a view that lists a page of
 * plans. */
const useEditLadder = (edit: Edit, done: (plan: PlanOverview) => string, failed: string) => {
  const transport = useTransport()
  const queryClient = useQueryClient()
  const { mutateAsync } = useMutation(setSelfServeLadder)

  return useCallback(
    async (plan: PlanOverview) => {
      try {
        const { plans } = await queryClient.fetchQuery(
          createQueryOptions(
            listPlans,
            {
              productFamilyLocalId: plan.productFamilyLocalId,
              pagination: { page: 0, perPage: LADDER_PAGE_SIZE },
            },
            { transport }
          )
        )
        await mutateAsync({
          productFamilyLocalId: plan.productFamilyLocalId,
          planIds: edit(
            ladderOf(plans).map(p => p.id),
            plan
          ),
        })
        await invalidateLadder(queryClient)
        toast.success(done(plan))
      } catch (error) {
        toast.error((error instanceof ConnectError && error.rawMessage) || failed)
      }
    },
    [transport, queryClient, mutateAsync, edit, done, failed]
  )
}

const added = (plan: PlanOverview) => `${plan.name} is on the pricing page`
const removed = (plan: PlanOverview) => `${plan.name} is off the pricing page`

/** Puts a plan last on its product line's pricing page. */
export const useAddToPricingPage = () =>
  useEditLadder(last, added, 'The plan could not be added to the pricing page')

/** Takes a plan off its product line's pricing page. */
export const useRemoveFromPricingPage = () =>
  useEditLadder(without, removed, 'The plan could not be removed from the pricing page')
