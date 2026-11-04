import { Navigate, useParams } from 'react-router-dom'

import { DemoDataOnboardingForm } from '@/features/onboarding/demoDataOnboardingForm'
import { useQuery } from '@/lib/connectrpc'
import { TenantEnvironmentEnum } from '@/rpc/api/tenants/v1/models_pb'
import { listTenants } from '@/rpc/api/tenants/v1/tenants-TenantsService_connectquery'

export const DemoDataOnboarding = () => {
  const { organizationSlug } = useParams()
  const tenantsQuery = useQuery(listTenants)

  if (!organizationSlug) {
    return <Navigate to="/" />
  }
  // Offered once: an organization that already has a sandbox goes straight in.
  if (tenantsQuery.data?.tenants.some(t => t.environment === TenantEnvironmentEnum.SANDBOX)) {
    return <Navigate to={`/${organizationSlug}`} />
  }

  return (
    <>
      <div className="w-full lg:w-1/2 xl:w-2/5 bg-[#111]">
        {!tenantsQuery.isLoading && (
          <DemoDataOnboardingForm organizationSlug={organizationSlug} />
        )}
      </div>
      <div className="hidden lg:block lg:w-1/2 xl:w-3/5 h-full">
        <div className="h-full pl-16 pt-16 flex justify-end">
          <img
            src="/img/onboarding/org.png"
            alt="demo data onboarding"
            className="h-full object-cover object-left"
          />
        </div>
      </div>
    </>
  )
}
