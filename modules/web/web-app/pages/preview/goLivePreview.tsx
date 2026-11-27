import { Form, InputFormField } from '@md/ui'
import { useState } from 'react'
import { useNavigate } from 'react-router'
import { z } from 'zod'

import {
  WizardActions,
  WizardBackButton,
  WizardColumn,
  WizardFieldGroup,
  WizardLayout,
  WizardPrimaryButton,
  WizardStepHeader,
  WizardStepper,
} from '@/components/wizard/Wizard'
import { useZodForm } from '@/hooks/useZodForm'

/**
 * Dev-only preview of the go-live flow's design ("Activate your account"), at /preview/go-live.
 *
 * The real flow lives in Meteroid Cloud; this page only shows the Wizard pieces with the fields
 * of the Figma concept, so the design can be reviewed here. Nothing is saved.
 */
const informationSchema = z.object({
  tradeName: z.string().min(1, 'Required'),
  billingEmail: z.union([z.literal(''), z.string().email()]),
  vatNumber: z.string(),
  addressLine1: z.string(),
  addressLine2: z.string(),
  zipCode: z.string(),
  city: z.string(),
  state: z.string(),
})

const PLAN_PLACEHOLDERS = ['Plan A', 'Plan B', 'Plan C']

export const GoLivePreview = () => {
  const navigate = useNavigate()
  const [step, setStep] = useState(0)

  const methods = useZodForm({
    schema: informationSchema,
    defaultValues: {
      tradeName: 'Meteroid',
      billingEmail: '',
      vatNumber: '',
      addressLine1: '',
      addressLine2: '',
      zipCode: '',
      city: '',
      state: '',
    },
  })

  return (
    <WizardLayout title="Activate your account" onClose={() => navigate('/')}>
      <WizardColumn>
        <WizardStepper steps={2} current={step} />
      </WizardColumn>

      {step === 0 ? (
        <WizardColumn>
          <WizardStepHeader
            title="Information"
            description="First, let's set up your business details. These will be used for billing and will appear on invoices."
          />
          <Form {...methods}>
            <form onSubmit={methods.handleSubmit(() => setStep(1))} className="flex flex-col gap-6">
              <div className="flex flex-col gap-4">
                <InputFormField
                  name="tradeName"
                  label="Trade name"
                  placeholder="Acme Inc."
                  control={methods.control}
                />
                <InputFormField
                  name="billingEmail"
                  label="Billing email"
                  type="email"
                  placeholder="billing@acme.com"
                  control={methods.control}
                />
                <InputFormField
                  name="vatNumber"
                  label="VAT number (Tax ID)"
                  placeholder="FR 123456789"
                  control={methods.control}
                />
                <WizardFieldGroup label="Company address">
                  <InputFormField
                    name="addressLine1"
                    placeholder="Address line 1"
                    aria-label="Address line 1"
                    control={methods.control}
                  />
                  <InputFormField
                    name="addressLine2"
                    placeholder="Address line 2"
                    aria-label="Address line 2"
                    control={methods.control}
                  />
                  <div className="grid grid-cols-2 gap-4">
                    <InputFormField
                      name="zipCode"
                      placeholder="Postal code"
                      aria-label="Postal code"
                      control={methods.control}
                    />
                    <InputFormField
                      name="city"
                      placeholder="City"
                      aria-label="City"
                      control={methods.control}
                    />
                  </div>
                  <InputFormField
                    name="state"
                    placeholder="State"
                    aria-label="State"
                    control={methods.control}
                  />
                </WizardFieldGroup>
              </div>

              <WizardActions>
                <WizardPrimaryButton type="submit">Continue</WizardPrimaryButton>
              </WizardActions>
            </form>
          </Form>
        </WizardColumn>
      ) : (
        <>
          <WizardColumn>
            <WizardStepHeader
              title="Choose a plan"
              description="Select a plan to create your Production tenant and start billing your customers."
            />
          </WizardColumn>

          {/* The plans are wider than the column: three 320px cards. */}
          <div className="mt-3 grid w-full max-w-[996px] grid-cols-1 gap-[18px] md:grid-cols-3">
            {PLAN_PLACEHOLDERS.map(plan => (
              <div key={plan} className="min-h-[492px] rounded-lg bg-[#1b1b1b] p-7">
                <h3 className="text-lg font-medium leading-5 text-[#f2f2f2]">{plan}</h3>
              </div>
            ))}
          </div>

          <WizardColumn className="mt-10">
            <WizardActions>
              <WizardBackButton onClick={() => setStep(0)} />
              <WizardPrimaryButton type="button" onClick={() => navigate('/')}>
                Skip
              </WizardPrimaryButton>
            </WizardActions>
          </WizardColumn>
        </>
      )}
    </WizardLayout>
  )
}
