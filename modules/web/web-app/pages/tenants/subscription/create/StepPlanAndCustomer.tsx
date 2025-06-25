import { Button, Form, GenericFormField } from '@ui/components'
import { useAtom } from 'jotai'
import { useWizard } from 'react-use-wizard'
import { z } from 'zod'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { CustomerSelect } from '@/features/customers/CustomerSelect'
import { SubscribablePlanVersionSelect } from '@/features/plans/SubscribablePlanVersionSelect'
import { CreateSubscriptionPriceComponents } from '@/features/subscriptions/pricecomponents/CreateSubscriptionPriceComponents'
import { useZodForm } from '@/hooks/useZodForm'
import { createSubscriptionAtom } from '@/pages/tenants/subscription/create/state'

// TODO confirm & reset form on leave
export const StepPlanAndCustomer = () => {
  const { nextStep } = useWizard()
  const [state, setState] = useAtom(createSubscriptionAtom)
  const methods = useZodForm({
    schema: schema,
    defaultValues: state,
  })
  const [customerId, planVersionId] = methods.watch(['customerId', 'planVersionId'])

  const onSubmit = async (data: z.infer<typeof schema>) => {
    setState({
      ...state,
      ...data,
    })
    nextStep()
  }

  console.log([customerId, planVersionId])

  return (
    <Form {...methods}>
      <form onSubmit={methods.handleSubmit(onSubmit)}>
        <PageSection
          className="fadeIn"
          header={{
            title: 'Plan & Customer',
            subtitle: 'Choose the owner of the subscription',
          }}
        >
          <div className="flex flex-col gap-4 max-w-xl">
            <GenericFormField
              control={methods.control}
              layout="horizontal"
              label="Plan"
              name="planVersionId"
              render={({ field }) => (
                <SubscribablePlanVersionSelect value={field.value} onChange={field.onChange} />
              )}
            />
            <GenericFormField
              control={methods.control}
              layout="horizontal"
              label="Customer"
              name="customerId"
              render={({ field }) => (
                <CustomerSelect value={field.value} onChange={field.onChange} />
              )}
            />
          </div>
        </PageSection>
        {planVersionId && customerId && (
          <>
            <PageSection
              className="fadeIn"
              header={{
                title: 'Pricing',
                subtitle: 'Configure the price components attached to this subscription',
                actions: (
                  <Button variant="secondary" disabled type="button">
                    Add a fee
                  </Button>
                ),
              }}
            >
              {/* <div>
                <Alert variant="destructive">
                  <AlertDescription>
                    !! WIP !! This UI only works for non-parametrized price component in this
                    iteration
                  </AlertDescription>
                </Alert>
              </div> */}
              <CreateSubscriptionPriceComponents planVersionId={planVersionId} />
            </PageSection>

            <div className="flex gap-2 justify-end">
              <Button type="submit" variant="primary">
                Next
              </Button>
            </div>
          </>
        )}
      </form>
    </Form>
  )
}

const schema = z.object({
  planVersionId: z.string(),
  customerId: z.string(),
})
