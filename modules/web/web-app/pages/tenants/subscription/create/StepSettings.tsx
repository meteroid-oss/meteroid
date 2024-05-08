import { useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import {
  Button,
  DatePicker,
  Form,
  GenericFormField,
  Label,
  RadioGroup,
  RadioGroupItem,
} from '@ui/components'
import { useAtom } from 'jotai'
import { useNavigate } from 'react-router-dom'
import { useWizard } from 'react-use-wizard'
import { toast } from 'sonner'
import { z } from 'zod'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { useZodForm } from '@/hooks/useZodForm'
import { mapDatev2 } from '@/lib/mapping'
import { createSubscriptionAtom } from '@/pages/tenants/subscription/create/state'
import {
  createSubscription,
  listSubscriptions,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

export const StepSettings = () => {
  const navigate = useNavigate()
  const { previousStep } = useWizard()
  const [state, setState] = useAtom(createSubscriptionAtom)
  const methods = useZodForm({
    schema: schema,
    defaultValues: state,
  })
  const queryClient = useQueryClient()
  const createSubscriptionMutation = useMutation(createSubscription, {
    onSuccess: async () => {
      queryClient.invalidateQueries({ queryKey: [listSubscriptions.service.typeName] })
    },
  })

  const onSubmit = async (data: z.infer<typeof schema>) => {
    setState({
      ...state,
      ...data,
    })

    // TOD missing quite a lot of properties
    await createSubscriptionMutation.mutateAsync({
      subscription: {
        planVersionId: state.planVersionId,
        customerId: state.customerId,
        billingStartDate: mapDatev2(data.fromDate),
        billingEndDate: data.toDate && mapDatev2(data.toDate),
        billingDay: data.billingDay === 'FIRST' ? 1 : data.fromDate.getDate(),
      },
    })
    toast.success('Subscription created')
    navigate('..')
  }

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <PageSection
          className="fadeIn"
          header={{
            title: 'Subscription',
            subtitle: 'When does it start?',
          }}
        >
          <div className="flex flex-col gap-4 max-w-xl">
            <GenericFormField
              control={methods.control}
              layout="horizontal"
              label="From date"
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
              layout="horizontal"
              label="To date"
              name="toDate"
              render={({ field }) => (
                <DatePicker
                  mode="single"
                  captionLayout="dropdown"
                  className="min-w-[12em]"
                  placeholder="optional"
                  date={field.value}
                  onSelect={field.onChange}
                />
              )}
            />
            <GenericFormField
              control={methods.control}
              layout="horizontal"
              label="Billing cycle"
              name="billingDay"
              render={({ field }) => (
                <RadioGroup
                  className="min-w-[24em]"
                  name={field.name}
                  value={field.value}
                  onValueChange={field.onChange}
                >
                  <div className="flex items-center space-x-4">
                    <RadioGroupItem value="FIRST" id="r1" />
                    <Label htmlFor="r1">1st of the month</Label>
                  </div>
                  <div className="flex items-center space-x-4">
                    <RadioGroupItem value="SUB_START_DAY" id="r2" />
                    <Label htmlFor="r2">Start date of the subscription</Label>
                  </div>
                </RadioGroup>
              )}
            />
          </div>
        </PageSection>

        <div className="flex gap-2 justify-end">
          <Button onClick={previousStep} variant="secondary">
            Back
          </Button>
          <Button type="submit" variant="primary">
            Create
          </Button>
        </div>
      </form>
    </Form>
  )
}

const schema = z
  .object({
    fromDate: z.date(),
    toDate: z.date().optional(),
    billingDay: z.enum(['FIRST', 'SUB_START_DAY']).default('FIRST'),
  })
  .refine(data => !data.toDate || data.toDate > data.fromDate, {
    message: 'Must be greater than the start date',
    path: ['toDate'],
  })
