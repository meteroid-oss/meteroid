import { ValidateEmailForm } from '@/features/auth/components/ValidateEmailForm'

import type { FunctionComponent } from 'react'

export const ValidateEmail: FunctionComponent = () => {
  return (
    <>
      <div className="font-medium text-xl -mb-0.5">Email address verified! </div>
      <div className="text-muted-foreground text-[13px] mb-1 leading-[18px]">
        Set your password for Meteroid to continue.
      </div>
      <ValidateEmailForm />
    </>
  )
}
