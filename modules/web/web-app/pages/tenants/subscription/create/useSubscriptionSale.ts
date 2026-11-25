import { useMemo } from 'react'

import {
  byPosition,
  inOption,
  newSubscriptionSale,
  optionHandle,
} from '@/features/subscriptions/pricecomponents/planOption'
import { useQuery } from '@/lib/connectrpc'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { getPlanWithVersionByVersionId } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'

import type { PlanOption } from '@/rpc/api/plans/v1/models_pb'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'

const NO_OPTIONS: PlanOption[] = []

export interface SubscriptionSale {
  options: PlanOption[]
  option?: PlanOption
  /** What `CreateSubscription.option` is sent: the option shown, so the server bills it. */
  handle?: string
  currency?: string
  /** The plan's charges as bought in the option, in the currency. */
  components?: PriceComponent[]
}

/** The option and currency the new subscription is sold in, and its charges' prices there. */
export const useSubscriptionSale = ({
  planVersionId,
  customerId,
  option,
}: {
  planVersionId?: string
  customerId?: string
  option?: string
}): SubscriptionSale => {
  const planQuery = useQuery(
    getPlanWithVersionByVersionId,
    { localId: planVersionId ?? '' },
    { enabled: Boolean(planVersionId) }
  )
  const componentsQuery = useQuery(
    listPriceComponents,
    { planVersionId: planVersionId ?? '' },
    { enabled: Boolean(planVersionId) }
  )
  const customerQuery = useQuery(
    getCustomerById,
    { id: customerId ?? '' },
    { enabled: Boolean(customerId) }
  )

  const version = planQuery.data?.plan?.version
  const raw = componentsQuery.data?.components
  const customerCurrency = customerQuery.data?.customer?.currency

  return useMemo(() => {
    const options = version ? byPosition(version.options) : NO_OPTIONS
    if (!version || !raw) return { options }
    const sale = newSubscriptionSale(options, raw, {
      customerCurrency,
      planCurrency: version.currency,
      handle: option,
    })
    return {
      options,
      option: sale.option,
      handle: sale.option && optionHandle(sale.option),
      currency: sale.currency,
      components: inOption(raw, sale.option, sale.currency),
    }
  }, [version, raw, customerCurrency, option])
}
