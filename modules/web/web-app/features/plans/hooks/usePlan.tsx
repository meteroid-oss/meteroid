import { disableQuery } from '@connectrpc/connect-query'

import { useQuery } from '@/lib/connectrpc'
import { PlanWithVersion } from '@/rpc/api/plans/v1/models_pb'
import {
  getPlanOverview,
  getPlanWithVersion,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { useTypedParams } from '@/utils/params'

const getVersionFilter = (planVersion?: string) => {
  if (!planVersion || planVersion === 'active') {
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

  const planOverview = usePlanOverview()

  // so if no version provided, we wait for planOverview to check if there is an active version, or a draft version

  let filter = null

  if (planVersion) {
    filter = getVersionFilter(planVersion)
  } else {
    if (planOverview) {
      if (planOverview.activeVersion) {
        filter = getVersionFilter('active')
      } else {
        filter = getVersionFilter('draft')
      }
    }
  }

  const planQuery = useQuery(
    getPlanWithVersion,
    planLocalId && filter ? { localId: planLocalId!, filter } : disableQuery
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
      : disableQuery
  )

  return data?.planOverview
}

export const useIsDraftVersion = () => {
  const { version } = usePlanWithVersion()

  return version?.isDraft === true
}
