import { Button } from '@md/ui'
import { useNavigate } from 'react-router-dom'

import { OrganizationOnboardingForm } from '@/features/onboarding/organizationOnboardingForm'
import { useQuery } from '@/lib/connectrpc'
import { getInstance } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'
import { me } from '@/rpc/api/users/v1/users-UsersService_connectquery'

export const OrganizationOnboarding = () => {
  const getInstanceQuery = useQuery(getInstance)
  const meQuery = useQuery(me)

  const navigate = useNavigate()

  if (
    getInstanceQuery?.data?.instanceInitiated &&
    !getInstanceQuery.data.multiOrganizationEnabled &&
    meQuery.data
  ) {
    if (meQuery?.data?.organizations.length >= 1) {
      return (
        <div className="p-10">
          <div>
            You cannot create more organizations on this instance. Please contact your account
            manager.
          </div>
          <div>
            <Button onClick={() => navigate('/')}>Back</Button>
          </div>
        </div>
      )
    }

    return (
      <div className="p-10">
        <div>
          You don&apos;t have access to this instance. Request an invite link to your admin.
        </div>
        <div>
          <Button onClick={() => navigate('/logout')}>Logout</Button>
        </div>
      </div>
    )
  }

  return (
    <>
      <div className="w-full lg:w-2/5 bg-[#111] rounded-lg lg:rounded-l-lg lg:rounded-r-none">
        <OrganizationOnboardingForm />
      </div>
      <div className="hidden lg:block lg:w-3/5 bg-[#313131] rounded-r-lg h-full">
        <div className="h-full pl-16 pt-16 flex justify-end">
          <img src="/img/onboarding/org.svg" alt="user onboarding" className="h-full" />
        </div>
      </div>
    </>
  )
}
