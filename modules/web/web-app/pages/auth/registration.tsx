import { useSearchParams } from 'react-router-dom'

import { Loader } from '@/features/auth/components/Loader'
import { RegistrationForm } from '@/features/auth/components/RegistrationForm'
import { useQuery } from '@/lib/connectrpc'
import { INVITE_TOKEN_KEY } from '@/pages/invite/acceptInvite'
import { getInstance } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'

import type { FunctionComponent } from 'react'

export const Registration: FunctionComponent = () => {
  const { data, isLoading } = useQuery(getInstance, undefined, {
    refetchOnMount: 'always',
  })

  const [searchParams] = useSearchParams()

  // Check URL params first, then sessionStorage
  const invite =
    searchParams.get('invite') ?? sessionStorage.getItem(INVITE_TOKEN_KEY) ?? undefined

  if (isLoading) {
    return <Loader />
  }

  if (data && data.instanceInitiated && !data.multiOrganizationEnabled && !invite) {
    return (
      <div className="text-center text-sm ">
        To join your organisation, request an invite link from your administrator
      </div>
    )
  }

  return <RegistrationForm invite={invite} />
}
