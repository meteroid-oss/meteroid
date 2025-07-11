import { useResetAtom } from 'jotai/utils'
import { Fragment, useEffect } from 'react'
import { Wizard } from 'react-use-wizard'

import PageHeading from '@/components/PageHeading/PageHeading'
import { StepPlanAndCustomer } from '@/pages/tenants/subscription/create/StepPlanAndCustomer'
import { StepReviewAndCreate } from '@/pages/tenants/subscription/create/StepReviewAndCreate'
import { StepSettings } from '@/pages/tenants/subscription/create/StepSettings'
import { createSubscriptionAtom } from '@/pages/tenants/subscription/create/state'

export const SubscriptionCreate = () => {
  const resetState = useResetAtom(createSubscriptionAtom)

  useEffect(() => {
    return () => {
      resetState()
    }
  }, [])

  return (
    <Fragment>
      <PageHeading>Create a new subscription</PageHeading>
      <div className="flex flex-col pt-8">
        <Wizard>
          <StepPlanAndCustomer />
          <StepSettings />
          <StepReviewAndCreate />
        </Wizard>
      </div>
    </Fragment>
  )
}
