import { Alert, cn } from '@md/ui'
import { useLocation } from 'react-router-dom'

import { LoginForm } from '@/features/auth/components/LoginForm'

export const Login = (): JSX.Element => {
  const location = useLocation()

  return location.state === 'accountCreated' ? (
    <div className={cn('flex flex-col space-y-4')}>
      <Alert>Your account has been created, you can now login!</Alert>
      <LoginForm />
    </div>
  ) : (
    <LoginForm />
  )
}
