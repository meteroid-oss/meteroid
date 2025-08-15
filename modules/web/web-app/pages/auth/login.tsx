import { Alert, cn } from '@md/ui'
import { useEffect } from "react";
import { useSearchParams , useLocation } from 'react-router-dom'
import { toast } from "sonner";

import { LoginForm } from '@/features/auth/components/LoginForm'

export const Login = (): JSX.Element => {
  const location = useLocation()
  const [searchParams] = useSearchParams()

  const error = searchParams.get('error')

  useEffect(() => {
    setTimeout(() => {
      error && toast.error(error, { id: 'login_url_error' })
    }, 1)
  }, [error])

  return location.state === 'accountCreated' ? (
    <div className={cn('flex flex-col space-y-4')}>
      <Alert>Your account has been created, you can now login!</Alert>
      <LoginForm/>
    </div>
  ) : (
    <LoginForm/>
  )
}
