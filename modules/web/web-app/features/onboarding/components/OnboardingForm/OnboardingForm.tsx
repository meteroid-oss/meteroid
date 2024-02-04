import { spaces } from '@md/foundation'
import { Button, Flex, FormItem, Input } from '@md/ui'
import { FunctionComponent } from 'react'

import { useZodForm } from '@/hooks/useZodForm'
import { schemas } from '@/lib/schemas'

interface OnboardingFormProps {
  nextStep: () => Promise<void>
}

const OnboardingForm: FunctionComponent<OnboardingFormProps> = ({ nextStep }) => {
  const methods = useZodForm({
    schema: schemas.organizations.completeOnboardingSchema,
    mode: 'onBlur',
    defaultValues: {
      organization: '',
      tenant: '',
      currency: 'EUR', // TODO
    },
  })

  return (
    <form
      onSubmit={methods.handleSubmit(async () => {
        nextStep()
      })}
    >
      <Flex direction="column" gap={spaces.space9}>
        <Flex direction="column" gap={spaces.space6}>
          <FormItem
            name="organization"
            label="Organisation name"
            error={methods.formState.errors.organization?.message}
          >
            <Input type="text" placeholder="ACME Inc." {...methods.register('organization')} />
          </FormItem>

          <FormItem
            name="tenant"
            label="Development tenant name"
            hint="You can create more tenants later"
            error={methods.formState.errors.tenant?.message}
          >
            <Input type="text" placeholder="development" {...methods.register('tenant')} />
          </FormItem>
        </Flex>

        <Button type="submit" variant="primary" fullWidth>
          Set up and continue
        </Button>
      </Flex>
    </form>
  )
}

export default OnboardingForm
