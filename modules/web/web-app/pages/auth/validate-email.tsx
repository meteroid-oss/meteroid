import { ValidateEmailForm } from '@/features/auth/ValidateEmailForm'
import type { FunctionComponent } from 'react'

export const ValidateEmail: FunctionComponent = () => {
  return (
    <>
      <div className="font-medium text-xl -mb-0.5">Check your inbox</div>
      <div className="text-muted-foreground text-[13px] mb-3 leading-[18px]">
        We’ve sent you a temporary login link. To continue, please check your inbox at:{' '}
        <span className="text-foreground">acme@gmail.com</span>
      </div>
      <ValidateEmailForm />
    </>
  )
}
