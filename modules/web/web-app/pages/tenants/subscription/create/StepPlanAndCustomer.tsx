import { Button, Label } from '@ui/components'
import { useAtom } from 'jotai'
import { useWizard } from 'react-use-wizard'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { SubscribablePlanVersionSelect } from '@/features/billing/plans/SubscribablePlanVersionSelect'
import { PriceComponentOverview } from '@/features/billing/plans/pricecomponents/PriceComponentOverview'
import { CustomerSelect } from '@/features/customers/CustomerSelect'
import { createSubscriptionAtom } from '@/pages/tenants/subscription/create/state'

export const StepPlanAndCustomer = () => {
  const { nextStep } = useWizard()
  const [state, setState] = useAtom(createSubscriptionAtom)

  return (
    <>
      <PageSection
        className="fadeIn"
        header={{
          title: 'Plan & Customer',
          subtitle: 'Choose the owner of the subscription',
        }}
      >
        <div className="flex flex-col gap-2">
          <Label className="flex items-center gap-3">
            <div className="w-[6em]">Plan</div>
            <SubscribablePlanVersionSelect
              value={state.planVersionId}
              onChange={v =>
                setState({
                  ...state,
                  planVersionId: v,
                })
              }
            />
          </Label>

          <Label className="flex items-center gap-3">
            <div className="w-[6em]">Customer</div>
            <CustomerSelect
              value={state.customerId}
              onChange={v =>
                setState({
                  ...state,
                  customerId: v,
                })
              }
            />
          </Label>
        </div>
      </PageSection>
      {state.planVersionId && state.customerId && (
        <>
          <PageSection
            className="fadeIn"
            header={{
              title: 'Pricing',
              subtitle: 'All price components of the selected plan',
            }}
          >
            <PriceComponentOverview planVersionId={state.planVersionId} />
          </PageSection>

          <div className="flex gap-2 justify-end">
            <Button onClick={nextStep} variant="primary">
              Next
            </Button>
          </div>
        </>
      )}
    </>
  )
}
