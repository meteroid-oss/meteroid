import { Alert , cn } from '@md/ui'
import { Navigate, useLocation } from 'react-router-dom'

import { Loader } from '@/features/auth/components/Loader'
import { LoginForm } from '@/features/auth/components/LoginForm'
import { useQuery } from '@/lib/connectrpc'
import { getInstance } from '@/rpc/api/instance/v1/instance-InstanceService_connectquery'

export const Login = (): JSX.Element => {
  const location = useLocation()
  const { data, isLoading } = useQuery(getInstance)

  if (isLoading) {
    return <Loader />
  }

  if (!isLoading && !data?.instance?.organizationId) {
    return <Navigate to="/registration" />
  }

  return (
    <div className={cn('flex flex-col space-y-4')}>
      {location.state === 'accountCreated' && (
        <Alert>Your account has been created, you can now login!</Alert>
      )}
      <LoginForm />
    </div>
  )
}
