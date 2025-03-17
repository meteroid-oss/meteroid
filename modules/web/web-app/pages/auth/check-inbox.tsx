import { Button } from '@ui/components'
import { useLocation, useNavigate } from 'react-router-dom'

import type { FunctionComponent } from 'react'

export const CheckInbox: FunctionComponent = () => {
  const { state } = useLocation()
  const navigate = useNavigate()

  return (
    <>
      <div className="font-medium text-xl -mb-0.5">Check your inbox</div>
      <div className="text-muted-foreground text-[13px] mb-3 leading-[18px]">
        We’ve sent you a temporary login link. To continue, please check your inbox at:{' '}
        <span className="text-foreground">{state}</span>
      </div>
      <Button variant="secondary" className="w-full" onClick={() => navigate('/registration')}>
        Back to Sign up
      </Button>
    </>
  )
}
