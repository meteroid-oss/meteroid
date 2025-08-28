import { Navigate, useParams } from 'react-router-dom'

import { useQuery } from '@/lib/connectrpc'
import { getPlanWithVersionByVersionId } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

// Component that resolves plan version ID and redirects to proper plan route
export const PlanVersionRedirect = () => {
  const { planVersionId, organizationSlug, tenantSlug } = useParams<{
    planVersionId: string
    organizationSlug: string
    tenantSlug: string
  }>()

  const planQuery = useQuery(
    getPlanWithVersionByVersionId,
    { localId: planVersionId ?? '' },
    { enabled: Boolean(planVersionId) }
  )

  if (planQuery.isLoading) {
    return (
      <div className="h-full flex items-center justify-center">
        <div className="text-sm text-muted-foreground">Loading plan...</div>
      </div>
    )
  }

  if (planQuery.error || !planQuery.data?.plan?.plan || !planQuery.data?.plan.version) {
    return <Navigate to={`/${organizationSlug}/${tenantSlug}/plans`} replace />
  }

  const planLocalId = planQuery.data.plan.plan.localId
  const versionNumber = planQuery.data.plan.version.version

  return (
    <Navigate
      to={`/${organizationSlug}/${tenantSlug}/plans/${planLocalId}/${versionNumber}`}
      replace
    />
  )
}
