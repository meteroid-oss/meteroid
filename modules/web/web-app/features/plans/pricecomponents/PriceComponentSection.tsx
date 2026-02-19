import { disableQuery } from '@connectrpc/connect-query'
import { Button } from '@md/ui'
import { useNavigate } from 'react-router-dom'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { useIsDraftVersion, usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { EditPriceComponent } from '@/features/plans/pricecomponents/EditPriceComponent'
import { PriceComponentCard } from '@/features/plans/pricecomponents/PriceComponentCard'
import { useEditedComponents } from '@/features/plans/pricecomponents/utils'
import { useQuery } from '@/lib/connectrpc'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'

export const PriceComponentSection = () => {
  const navigate = useNavigate()

  const planWithVersion = usePlanWithVersion()
  const editedComponents = useEditedComponents()
  const isDraft = useIsDraftVersion()

  const priceComponents = useQuery(
    listPriceComponents,
    planWithVersion?.version
      ? { planVersionId: planWithVersion.version.id }
      : disableQuery
  )?.data?.components

  return (
    <PageSection
      header={{
        title: 'Pricing',
        subtitle: 'The price components for your plan in your main currency',
        actions: isDraft ? (
          <Button
            variant="outline"
            onClick={() => navigate('./add-component')}
            className="py-1.5"
          >
            + Add a price component
          </Button>
        ) : null,
      }}
    >
      <div className="grid gap-y-4">
        {priceComponents?.map(component =>
          isDraft && editedComponents?.find(id => id === component.id) ? (
            <EditPriceComponent component={component} key={component.id} />
          ) : (
            <PriceComponentCard component={component} key={component.id} />
          )
        )}
        {priceComponents?.length === 0 && (
          <span>No price components</span>
        )}
      </div>
    </PageSection>
  )
}
