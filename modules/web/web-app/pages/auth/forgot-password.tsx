import { FunctionComponent } from 'react'

import { ForgotPasswordForm } from '@/features/auth/components/ForgotPasswordForm'

export const ForgotPassword: FunctionComponent = () => (
  <>
    <div className="font-medium text-xl -mb-0.5">Forgot password?</div>
    <div className="text-muted-foreground text-[13px] mb-3 leading-[18px]">
      Enter your email address and we will send you instructions to reset your password.
    </div>
    <ForgotPasswordForm />
  </>
)
