import { PriceComponentCard } from '@/features/billing/plans/pricecomponents/PriceComponentCard'
import { useQuery } from '@/lib/connectrpc'
import { mapFeeType } from '@/lib/mapping/feesFromGrpc'
import { PriceComponent } from '@/lib/schemas/plans'
import { PlanVersion } from '@/rpc/api/plans/v1/models_pb'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1_2/pricecomponents-PriceComponentsService_connectquery'

export const PriceComponentOverview = ({ planVersionId }: { planVersionId: PlanVersion['id'] }) => {
  const priceComponents = useQuery(
    listPriceComponents,
    {
      planVersionId: planVersionId ?? '',
    },
    { enabled: Boolean(planVersionId) }
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
    <div className="grid gap-y-4">
      {priceComponents?.map(priceComponent => (
        <PriceComponentCard component={priceComponent} key={priceComponent.id} />
      ))}
      {!priceComponents?.length && <span>No price components</span>}
    </div>
  )
}
