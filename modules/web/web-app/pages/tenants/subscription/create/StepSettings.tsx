import {
  Button,
  Card,
  CardContent,
  CardHeader,
  CardTitle,
  DatePicker,
  Form,
  GenericFormField,
  InputFormField,
  Label,
  RadioGroup,
  RadioGroupItem,
  SelectFormField,
  SelectItem,
  TextareaFormField,
} from '@ui/components'
import { useAtom } from 'jotai'
import { useWizard } from 'react-use-wizard'
import { z } from 'zod'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { useZodForm } from '@/hooks/useZodForm'
import { createSubscriptionAtom } from '@/pages/tenants/subscription/create/state'
import { ActivationCondition } from '@/rpc/api/subscriptions/v1/models_pb'

const activationConditionToString = (
  condition: ActivationCondition
): 'ON_START' | 'ON_CHECKOUT' | 'MANUAL' => {
  switch (condition) {
    case ActivationCondition.ON_START:
      return 'ON_START'
    case ActivationCondition.ON_CHECKOUT:
      return 'ON_CHECKOUT'
    case ActivationCondition.MANUAL:
      return 'MANUAL'
    default:
      return 'ON_START'
  }
}

const activationConditionFromString = (
  condition: 'ON_START' | 'ON_CHECKOUT' | 'MANUAL'
): ActivationCondition => {
  switch (condition) {
    case 'ON_START':
      return ActivationCondition.ON_START
    case 'ON_CHECKOUT':
      return ActivationCondition.ON_CHECKOUT
    case 'MANUAL':
      return ActivationCondition.MANUAL
    default:
      return ActivationCondition.ON_START
  }
}

export const StepSettings = () => {
  const { previousStep, nextStep } = useWizard()
  const [state, setState] = useAtom(createSubscriptionAtom)
  const methods = useZodForm({
    schema: schema,
    defaultValues: {
      fromDate: state.startDate,
      toDate: state.endDate,
      billingDay: state.billingDay,
      trialDuration: state.trialDuration,
      activationCondition: state.activationCondition
        ? activationConditionToString(state.activationCondition)
        : 'ON_START', // TODO error is here, somhow state.activationCondition can be undefined when going back
      netTerms: state.netTerms,
      invoiceMemo: state.invoiceMemo,
      invoiceThreshold: state.invoiceThreshold,
    },
  })

  const onSubmit = async (data: z.infer<typeof schema>) => {
    setState({
      ...state,
      startDate: data.fromDate,
      endDate: data.toDate,
      billingDay: data.billingDay,
      trialDuration: data.trialDuration,
      activationCondition: activationConditionFromString(data.activationCondition),
      netTerms: data.netTerms,
      invoiceMemo: data.invoiceMemo,
      invoiceThreshold: data.invoiceThreshold,
    })
    nextStep()
  }

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)} className="space-y-6">
        {/* Subscription Timing */}
        <PageSection
          className="fadeIn"
          header={{
            title: 'Subscription Timeline',
            subtitle: 'Configure when the subscription starts and its lifecycle',
          }}
        >
          <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Subscription Dates</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <GenericFormField
                  control={methods.control}
                  layout="vertical"
                  label="Start date"
                  name="fromDate"
                  render={({ field }) => (
                    <DatePicker
                      mode="single"
                      captionLayout="dropdown"
                      className="min-w-[12em]"
                      date={field.value}
                      onSelect={field.onChange}
                    />
                  )}
                />
                <GenericFormField
                  control={methods.control}
                  layout="vertical"
                  label="End date (optional)"
                  name="toDate"
                  render={({ field }) => (
                    <DatePicker
                      mode="single"
                      captionLayout="dropdown"
                      className="min-w-[12em]"
                      placeholder="No end date"
                      date={field.value}
                      onSelect={field.onChange}
                    />
                  )}
                />
                <InputFormField
                  name="trialDuration"
                  label="Trial Duration (days)"
                  type="number"
                  containerClassName="hidden"
                  placeholder="7"
                  control={methods.control}
                />
                {/* TODO */}
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle className="text-base">Billing Configuration</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <GenericFormField
                  control={methods.control}
                  layout="vertical"
                  label="Billing cycle"
                  name="billingDay"
                  render={({ field }) => (
                    <RadioGroup
                      name={field.name}
                      value={field.value}
                      onValueChange={field.onChange}
                    >
                      <div className="flex items-center space-x-4">
                        <RadioGroupItem value="FIRST" id="r1" />
                        <Label htmlFor="r1" className="font-normal">
                          1st of the month
                        </Label>
                      </div>
                      <div className="flex items-center space-x-4">
                        <RadioGroupItem value="SUB_START_DAY" id="r2" />
                        <Label htmlFor="r2" className="font-normal">
                          Anniversary date of the subscription
                        </Label>
                      </div>
                    </RadioGroup>
                  )}
                />
                <InputFormField
                  name="netTerms"
                  label="Net Terms (days)"
                  type="number"
                  placeholder="30"
                  control={methods.control}
                />
              </CardContent>
            </Card>
          </div>
        </PageSection>

        {/* Advanced Settings */}
        <PageSection
          header={{
            title: 'Advanced Settings',
            subtitle: 'Configure activation conditions and invoice details',
          }}
        >
          <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Activation & Lifecycle</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <SelectFormField
                  name="activationCondition"
                  label="Activation Condition"
                  placeholder="Select when to activate"
                  control={methods.control}
                >
                  <SelectItem value="ON_START">On Start Date</SelectItem>
                  <SelectItem value="ON_CHECKOUT">On Checkout</SelectItem>
                  <SelectItem value="MANUAL">Manual Activation</SelectItem>
                </SelectFormField>
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle className="text-base">Invoice Customization</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <TextareaFormField
                  name="invoiceMemo"
                  label="Invoice Memo"
                  placeholder="Custom note for invoices..."
                  control={methods.control}
                  rows={3}
                />
                {/* <InputFormField
                  name="invoiceThreshold"
                  label="Invoice Threshold"
                  placeholder="100.00"
                  control={methods.control}
                /> */}
              </CardContent>
            </Card>
          </div>
        </PageSection>

        <div className="flex gap-2 justify-end">
          <Button onClick={previousStep} variant="secondary">
            Back
          </Button>
          <Button type="submit">Next: Summary</Button>
        </div>
      </form>
    </Form>
  )
}

const schema = z
  .object({
    fromDate: z.date(),
    toDate: z.date().optional(),
    billingDay: z.enum(['FIRST', 'SUB_START_DAY']).default('SUB_START_DAY'),
    trialDuration: z.number().min(0).optional(),
    activationCondition: z.enum(['ON_START', 'ON_CHECKOUT', 'MANUAL']),
    netTerms: z.number().min(0),
    invoiceMemo: z.string().optional(),
    invoiceThreshold: z.string().optional(),
  })
  .refine(data => !data.toDate || data.toDate > data.fromDate, {
    message: 'Must be greater than the start date',
    path: ['toDate'],
  })
