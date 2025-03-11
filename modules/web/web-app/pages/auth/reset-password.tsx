import { ResetPasswordForm } from '@/features/auth/components/ResetPasswordForm'
import type { FunctionComponent } from 'react'

export const ResetPassword: FunctionComponent = () => {
  return (
    <>
      <div className="font-medium text-xl -mb-0.5">Change your password</div>
      <div className="text-muted-foreground text-[13px] mb-1 leading-[18px]">
        Set your new password for your Meteroid account.
      </div>
      <ResetPasswordForm />
    </>
  )
}
