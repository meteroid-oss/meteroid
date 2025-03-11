import { useSearchParams } from 'react-router-dom'

import { Loader } from '@/features/auth/components/Loader'
import { RegistrationForm } from '@/features/auth/components/RegistrationForm'
import { useQuery } from '@/lib/connectrpc'
import { getInstance } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'

import type { FunctionComponent } from 'react'

export const Registration: FunctionComponent = () => {
  const { data, isLoading } = useQuery(getInstance, undefined, {
    refetchOnMount: 'always',
  })

  const [searchParams] = useSearchParams()

  const invite = searchParams.get('invite') ?? undefined

  if (isLoading) {
    return <Loader />
  }

  if (data && data.instanceInitiated && !data.multiOrganizationEnabled && !invite) {
    return (
      <div className="text-center">
        To join your organisation, request an invite link from your administrator
      </div>
    )
  }

  return <RegistrationForm invite={invite} />
}
