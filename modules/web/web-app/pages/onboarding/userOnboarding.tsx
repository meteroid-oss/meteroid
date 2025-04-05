import { Navigate } from 'react-router-dom'

import { UserOnboardingForm } from '@/features/onboarding/userOnboardingForm'
import { useQuery } from '@/lib/connectrpc'
import { me } from '@/rpc/api/users/v1/users-UsersService_connectquery'

export const UserOnboarding = () => {
  const meQuery = useQuery(me)
  if (meQuery.data?.user?.onboarded) {
    if (meQuery.data?.organizations.length >= 1) {
      // this was an invite & the user is new, so he has a single org
      return <Navigate to={`/${meQuery.data.organizations[0].slug}`} />
    } else {
      return <Navigate to="/onboarding/organization" />
    }
  }

  return (
    <>
      <div className="w-full lg:w-2/5 bg-[#111] rounded-lg lg:rounded-l-lg lg:rounded-r-none">
        <UserOnboardingForm />
      </div>
      <div className="hidden lg:block lg:w-3/5 bg-[#313131] rounded-r-lg h-full">
        <div className="h-full pl-16 py-16 flex justify-end">
          <img src="/img/onboarding/user.svg" alt="user onboarding" className="h-full" />
        </div>
      </div>
    </>
  )
}
