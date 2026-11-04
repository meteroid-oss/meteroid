import { Badge } from '@md/ui'

import { MinimumCommitmentDisplay, productScopeOptions } from '@/features/billing/minimumCommitment'
import { useQuery } from '@/lib/connectrpc'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import { SubscriptionMinimum } from '@/rpc/api/subscriptions/v1/models_pb'

interface Props {
  planVersionId: string
  currency: string
  minimum?: SubscriptionMinimum
}

/**
 * The minimum-commitment line of a live subscription's contract terms. Changing a commitment is an
 * amendment, so it is edited only through the Amend Subscription flow.
 */
export const SubscriptionMinimumCommitment = ({ planVersionId, currency, minimum }: Props) => {
  const componentsQuery = useQuery(listPriceComponents, { planVersionId })
  const components = productScopeOptions(componentsQuery.data?.components ?? [])

  return (
    <MinimumCommitmentDisplay
      currency={currency}
      components={components}
      commitment={minimum?.commitment}
      badge={
        minimum &&
        (minimum.isOverride ? (
          <Badge variant="default" size="sm">
            Override
          </Badge>
        ) : (
          <Badge variant="outline" size="sm">
            From plan
          </Badge>
        ))
      }
    />
  )
}
