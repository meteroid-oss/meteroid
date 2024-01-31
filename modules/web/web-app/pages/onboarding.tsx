import { Wizard } from '@md/ui'
import { type FunctionComponent } from 'react'

import { Step1, Step2, Step3, Step4 } from '@/features/onboarding'
import WizardHeader from '@/features/onboarding/components/WizardHeader/WizardHeader'

export const Onboarding: FunctionComponent = () => {
  // TODO : onboarding should be PER ORGANIZATION, not per user ! Let's keep things separated

  return (
    <Wizard header={<WizardHeader />}>
      <Step1 />
      <Step2 />
      <Step3 />
      <Step4 />
    </Wizard>
  )
}

export default Onboarding
