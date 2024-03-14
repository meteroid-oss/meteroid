import { Label } from '@ui/components'
import { Fragment, useState } from 'react'

import PageHeading from '@/components/PageHeading/PageHeading'
import { TenantPageLayout } from '@/components/layouts'
import { PageSection } from '@/components/layouts/shared/PageSection'
import { PlanSelect } from '@/features/billing/plans/PlanSelect'
import { PriceComponentOverview } from '@/features/billing/plans/pricecomponents/PriceComponentOverview'
import { CustomerSelect } from '@/features/customers/CustomerSelect'

export const SubscriptionCreate = () => {
  const [customerId, setCustomerId] = useState<string>()
  const [planExternalId, setPlanExternalId] = useState<string>()

  return (
    <Fragment>
      <TenantPageLayout title="Create subscription">
        <PageHeading>Create a new subscription</PageHeading>
        <div className="flex flex-col pt-8">
          <PageSection
            header={{
              title: 'Plan & Customer',
              subtitle: 'Choose the owner of the subscription',
            }}
          >
            <div className="grid grid-cols-2 items-center">
              <Label className="flex items-center gap-3">
                Plan
                <PlanSelect value={planExternalId} onChange={setPlanExternalId} />
              </Label>
              <Label className="flex items-center gap-3">
                Customer
                <CustomerSelect value={customerId} onChange={setCustomerId} />
              </Label>
            </div>
          </PageSection>
          {planExternalId && customerId && (
            <PageSection
              header={{
                title: 'Pricing',
                subtitle: 'All price components of the selected plan',
              }}
            >
              <PriceComponentOverview planExternalId={planExternalId} />
            </PageSection>
          )}
        </div>
      </TenantPageLayout>
    </Fragment>
  )
}
