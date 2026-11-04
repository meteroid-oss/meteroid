import { MinimumCommitmentDisplay, productScopeOptions } from '@/features/billing/minimumCommitment'
import { useQuery } from '@/lib/connectrpc'
import { PlanVersion } from '@/rpc/api/plans/v1/models_pb'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'

export const PlanMinimumCommitmentDisplay = ({ planVersion }: { planVersion: PlanVersion }) => {
  const componentsQuery = useQuery(listPriceComponents, { planVersionId: planVersion.id })

  return (
    <MinimumCommitmentDisplay
      currency={planVersion.currency}
      components={productScopeOptions(componentsQuery.data?.components ?? [])}
      commitment={planVersion.minimumCommitment}
    />
  )
}
