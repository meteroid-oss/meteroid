import { spaces } from '@md/foundation'
import { Alert, Flex, useWizard, Wizard } from '@md/ui'

import OnboardingForm from '@/features/onboarding/components/OnboardingForm'
import StepTitle from '@/features/onboarding/components/StepTitle/StepTitle'
import WizardWrapper from '@/features/onboarding/components/WizardWrapper/WizardWrapper'

import type { FunctionComponent } from 'react'

export const Step1: FunctionComponent = () => {
  const { previousStep, nextStep } = useWizard()
  return (
    <Wizard.AnimatedStep previousStep={previousStep}>
      <WizardWrapper>
        <StepTitle>
          Welcome!
          <br />
          Let&apos;s create your organisation
        </StepTitle>

        <Flex direction="column" gap={spaces.space9}>
          <OnboardingForm nextStep={nextStep} />
          <Alert title="What is a tenant?">
            A tenant is a dedicated and isolated environment. It provides a separate space where you
            can build, test, refine and deploy your billing infrastructure without impacting the
            other environments.
          </Alert>
        </Flex>
      </WizardWrapper>
    </Wizard.AnimatedStep>
  )
}
