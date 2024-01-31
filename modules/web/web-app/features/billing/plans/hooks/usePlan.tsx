import { disableQuery } from '@connectrpc/connect-query'

import { useQuery } from '@/lib/connectrpc'
import { getPlanByExternalId } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { useTypedParams } from '@/utils/params'

export const usePlan = () => {
  const { planExternalId } = useTypedParams<{ planExternalId: string }>()
  const planQuery = useQuery(
    getPlanByExternalId,
    planExternalId ? { externalId: planExternalId! } : disableQuery
  )

  return planQuery
}
