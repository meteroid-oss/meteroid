import { spaces } from '@md/foundation'
import {
  Button,
  Checkbox,
  CheckboxFormItem,
  Flex,
  FormItem,
  Textarea,
  Wizard,
  useWizard,
} from '@md/ui'
import { useState, type FunctionComponent } from 'react'
import { useNavigate } from 'react-router-dom'

import StepTitle from '@/features/onboarding/components/StepTitle'
import WizardWrapper from '@/features/onboarding/components/WizardWrapper/WizardWrapper'

export const Step4: FunctionComponent = () => {
  const { previousStep } = useWizard()

  const [, setEmails] = useState<string[]>([])

  // TODO

  // const detailsForm = z.object({
  //   emails: z.array(z.string())
  // })

  // const methods = useZodForm({
  //   schema: detailsForm,
  //   defaultValues: {
  //     emails
  //   }
  // })

  const navigate = useNavigate()

  const onSubmit = () => {
    navigate('/')
  }

  return (
    <Wizard.AnimatedStep previousStep={previousStep}>
      <WizardWrapper>
        <StepTitle>Invite your team (optional)</StepTitle>
        <Flex direction="column" gap={spaces.space9}>
          <Flex direction="column" gap={spaces.space6}>
            <FormItem
              name="emails"
              label="Emails"
              hint="Separate the emails with a comma. You can add more later on."
              optional
            >
              <Textarea
                placeholder="john@meteroid.io, jane@meteroid.io"
                onChange={e => setEmails(e.target.value.split(','))}
              />
            </FormItem>
            <CheckboxFormItem
              name="everyone"
              label="Let anyone at meteroid.io join this organisation"
            >
              <Checkbox id="everyone" />
            </CheckboxFormItem>
          </Flex>
          <Flex direction="row" align="center" justify="space-between">
            <Button variant="tertiary" onClick={previousStep}>
              Back
            </Button>
            <Button variant="primary" onClick={onSubmit}>
              {/*  TODO what is that ?  */}
              Create organization
            </Button>
          </Flex>
        </Flex>
      </WizardWrapper>
    </Wizard.AnimatedStep>
  )
}
