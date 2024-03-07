import { PriceComponentCard } from '@/features/billing/plans/pricecomponents/PriceComponentCard'
import { useQuery } from '@/lib/connectrpc'
import { mapFeeType } from '@/lib/mapping/feesFromGrpc'
import { PriceComponent } from '@/lib/schemas/plans'
import { Plan } from '@/rpc/api/plans/v1/models_pb'
import { getPlanByExternalId } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'

export const PriceComponentOverview = ({
  planExternalId,
}: {
  planExternalId: Plan['externalId']
}) => {
  const getPlanQuery = useQuery(
    getPlanByExternalId,
    {
      externalId: planExternalId ?? '',
    },
    { enabled: Boolean(planExternalId) }
  )

  const priceComponents = useQuery(
    listPriceComponents,
    {
      planVersionId: getPlanQuery?.data?.planDetails?.currentVersion?.id ?? '',
    },
    { enabled: Boolean(getPlanQuery?.data?.planDetails?.currentVersion?.id) }
  )?.data?.components?.map(
    c =>
      ({
        id: c.id,
        name: c.name,
        productItem: c.productItem,
        fee: c.feeType ? mapFeeType(c.feeType) : undefined,
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
