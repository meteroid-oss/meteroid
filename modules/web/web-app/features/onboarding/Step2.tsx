import { spaces } from '@md/foundation'
import { Button, Flex, FormItem, Input, Wizard, useWizard } from '@md/ui'
import { z } from 'zod'

import AvatarUpload from '@/features/onboarding/components/AvatarUpload/AvatarUpload'
import NewsletterSubscription from '@/features/onboarding/components/NewsletterSubscription/NewsletterSubscription'
import StepTitle from '@/features/onboarding/components/StepTitle'
import WizardWrapper from '@/features/onboarding/components/WizardWrapper/WizardWrapper'
import { useZodForm } from '@/hooks/useZodForm'

import type { FunctionComponent } from 'react'

export const Step2: FunctionComponent = () => {
  const { previousStep, nextStep } = useWizard()

  const detailsForm = z.object({
    firstName: z.string().nonempty('First name is required').max(256),
    lastName: z.string().nonempty('Last name is required').max(256),
  })

  const methods = useZodForm({
    schema: detailsForm,
  })

  const firstName = methods.watch('firstName')
  const lastName = methods.watch('lastName')
  const firstNameInitial = firstName?.charAt(0) || 'J'
  const lastNameInitial = lastName?.charAt(0) || 'D'
  const initials = `${firstNameInitial}${lastNameInitial}`

  return (
    <Wizard.AnimatedStep previousStep={previousStep}>
      <WizardWrapper>
        <StepTitle>Let&apos;s get to know you</StepTitle>
        <Flex direction="column" gap={spaces.space9}>
          <Flex direction="column" gap={spaces.space6}>
            <AvatarUpload initials={initials} />
            <FormItem
              name="firstName"
              label="First name"
              error={methods.formState.errors.firstName?.message}
            >
              <Input type="text" placeholder="John" {...methods.register('firstName')} />
            </FormItem>
            <FormItem
              name="lastName"
              label="Last name"
              error={methods.formState.errors.lastName?.message}
            >
              <Input type="text" placeholder="Doe" {...methods.register('lastName')} />
            </FormItem>
          </Flex>
          <Flex direction="column" gap={spaces.space9}>
            <NewsletterSubscription />

            <Button variant="primary" fullWidth onClick={() => nextStep()}>
              Continue
            </Button>
          </Flex>
        </Flex>
      </WizardWrapper>
    </Wizard.AnimatedStep>
  )
}
