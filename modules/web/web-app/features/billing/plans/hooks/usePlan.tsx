import { disableQuery } from '@connectrpc/connect-query'

import { useQuery } from '@/lib/connectrpc'
import { getPlanByLocalId } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { useTypedParams } from '@/utils/params'

export const usePlan = () => {
  const { planLocalId } = useTypedParams<{ planLocalId: string }>()
  const planQuery = useQuery(
    getPlanByLocalId,
    planLocalId ? { localId: planLocalId! } : disableQuery
  )

  return planQuery
}
