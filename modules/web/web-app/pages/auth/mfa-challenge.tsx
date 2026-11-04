import { MfaChallengeForm } from '@/features/auth/components/MfaChallengeForm'

import type { FunctionComponent } from 'react'


export const MfaChallenge: FunctionComponent = () => {
  return (
    <>
      <div className="font-medium text-xl -mb-0.5">Two-factor authentication</div>
      <div className="text-muted-foreground text-[13px] mb-1 leading-[18px]">
        Verify your identity to finish signing in.
      </div>
      <MfaChallengeForm />
    </>
  )
}
