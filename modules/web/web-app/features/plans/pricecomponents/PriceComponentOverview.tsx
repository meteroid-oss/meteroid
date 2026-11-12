import { PriceComponentCard } from '@/features/plans/pricecomponents/PriceComponentCard'
import { ComponentRefsProvider } from '@/features/plans/pricecomponents/componentRefs'
import { useQuery } from '@/lib/connectrpc'
import { PlanVersion } from '@/rpc/api/plans/v1/models_pb'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'

export const PriceComponentOverview = ({ planVersionId }: { planVersionId: PlanVersion['id'] }) => {
  const list = useQuery(
    listPriceComponents,
    {
      planVersionId: planVersionId ?? '',
    },
    { enabled: Boolean(planVersionId) }
  )?.data
  const priceComponents = list?.components

  return (
    <ComponentRefsProvider list={list}>
      <div className="grid gap-y-4">
        {priceComponents?.map(priceComponent => (
          <PriceComponentCard component={priceComponent} key={priceComponent.id} />
        ))}
        {!priceComponents?.length && <span>No price components</span>}
      </div>
    </ComponentRefsProvider>
  )
}
