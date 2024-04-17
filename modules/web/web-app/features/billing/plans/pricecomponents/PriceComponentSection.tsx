import { disableQuery } from '@connectrpc/connect-query'
import { Button } from '@md/ui'
import { useNavigate } from 'react-router-dom'

import { PageSection } from '@/components/layouts/shared/PageSection'
import {
  CreatePriceComponent,
  EditPriceComponent,
} from '@/features/billing/plans/pricecomponents/EditPriceComponentCard'
import { PriceComponentCard } from '@/features/billing/plans/pricecomponents/PriceComponentCard'
import {
  useIsDraftVersion,
  useAddedComponents,
  useEditedComponents,
  usePlanOverview,
} from '@/features/billing/plans/pricecomponents/utils'
import { useQuery } from '@/lib/connectrpc'
import { mapFeeType } from '@/lib/mapping/feesFromGrpc'
import { PriceComponent } from '@/lib/schemas/plans'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1_2/pricecomponents-PriceComponentsService_connectquery'

// TODO Provider
// also TODO, save the state instead of the id ?

export const PriceComponentSection = () => {
  const navigate = useNavigate()

  const overview = usePlanOverview()

  const addedComponents = useAddedComponents()
  const editedComponens = useEditedComponents()

  const isDraft = useIsDraftVersion()

  const priceComponents = useQuery(
    listPriceComponents,
    overview?.planVersionId
      ? {
          planVersionId: overview.planVersionId,
        }
      : disableQuery
  )?.data?.components?.map(
    c =>
      ({
        id: c.id,
        name: c.name,
        fee: c.fee ? mapFeeType(c.fee.feeType) : undefined,
        productItemId: c.productItemId,
      }) as PriceComponent
  )

  return (
    <PageSection
      header={{
        title: 'Pricing',
        subtitle: 'The price components for your plan in your main currency',
        actions: isDraft ? (
          <>
            <Button
              variant="outline"
              onClick={() => {
                navigate('./add-component')
              }}
              className="py-1.5  "
            >
              + Add a price component
            </Button>
          </>
        ) : null,
      }}
    >
      <div className="grid gap-y-4">
        {priceComponents?.map(priceComponent =>
          isDraft && editedComponens?.find(id => id === priceComponent.id) ? (
            <EditPriceComponent component={priceComponent} key={priceComponent.id} />
          ) : (
            <PriceComponentCard component={priceComponent} key={priceComponent.id} />
          )
        )}
        {isDraft &&
          addedComponents?.map(({ ref, component }) => (
            <CreatePriceComponent component={component} createRef={ref} key={ref} />
          ))}
        {priceComponents?.length === 0 && addedComponents?.length === 0 && (
          <span>No price components</span>
        )}
      </div>
    </PageSection>
  )
}
