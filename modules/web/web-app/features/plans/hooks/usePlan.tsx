import { create } from '@bufbuild/protobuf'
import { createConnectQueryKey, skipToken, useTransport } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { useCallback } from 'react'

import { usePrefetch } from '@/hooks/usePrefetch'
import { useQuery } from '@/lib/connectrpc'
import { PlanOverview, PlanWithVersion } from '@/rpc/api/plans/v1/models_pb'
import {
  getPlanOverview,
  getPlanWithVersion,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { GetPlanOverviewResponseSchema } from '@/rpc/api/plans/v1/plans_pb'
import { useTypedParams } from '@/utils/params'

const getVersionFilter = (planVersion?: string) => {
  if (!planVersion) {
    // Without a version in the URL, the plan opens on its active version, or on its draft if it
    // was never published: the server picks, so the version doesn't wait on the overview.
    return {
      case: 'activeOrDraft' as const,
      value: {},
    }
  } else if (planVersion === 'active') {
    return {
      case: 'active' as const,
      value: {},
    }
  } else if (planVersion === 'draft') {
    return {
      case: 'draft' as const,
      value: {},
    }
  } else {
    return {
      case: 'version' as const,
      value: parseInt(planVersion),
    }
  }
}

export const usePlanWithVersion = () => {
  const { planLocalId, planVersion } = useTypedParams<{
    planLocalId: string
    planVersion?: string
  }>()

  const planQuery = useQuery(
    getPlanWithVersion,
    planLocalId ? { localId: planLocalId, filter: getVersionFilter(planVersion) } : skipToken
  )

  const data = planQuery.data?.plan ?? ({} as PlanWithVersion)

  return {
    isLoading: planQuery.isLoading,
    version: data.version,
    plan: data.plan,
  }
}

export const usePlanOverview = () => {
  const { planLocalId } = useTypedParams<{
    planLocalId: string
  }>()

  const { data } = useQuery(
    getPlanOverview,
    planLocalId
      ? {
          localId: planLocalId,
        }
      : skipToken
  )

  return data?.planOverview
}

export const useIsDraftVersion = () => {
  const { version } = usePlanWithVersion()

  return version?.isDraft === true
}

/**
 * Warms what the plan page loads first. The list row already is the plan's overview, so it seeds
 * that entry (as fresh as the list) for the header, and the version is prefetched.
 */
export const usePrefetchPlan = () => {
  const queryClient = useQueryClient()
  const transport = useTransport()
  const prefetch = usePrefetch()

  return useCallback(
    (overview: PlanOverview, listUpdatedAt: number) => {
      const localId = overview.localId
      const overviewKey = createConnectQueryKey({
        schema: getPlanOverview,
        input: { localId },
        transport,
        cardinality: 'finite',
      })
      if (queryClient.getQueryData(overviewKey) === undefined) {
        queryClient.setQueryData(
          overviewKey,
          create(GetPlanOverviewResponseSchema, { planOverview: overview }),
          { updatedAt: listUpdatedAt }
        )
      }
      prefetch(getPlanWithVersion, { localId, filter: getVersionFilter() })
    },
    [queryClient, transport, prefetch]
  )
}
