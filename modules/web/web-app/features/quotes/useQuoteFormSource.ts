import { useQuery } from '@/lib/connectrpc'
import { PlanStatus } from '@/rpc/api/plans/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import { getQuote } from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'

/** Loads what's needed to pre-fill the quote form from an existing quote. */
export const useQuoteFormSource = (quoteId: string | undefined) => {
  const quoteQuery = useQuery(getQuote, { id: quoteId ?? '' }, { enabled: Boolean(quoteId) })
  const quote = quoteQuery.data?.quote
  const planVersionId = quote?.quote?.planVersionId

  const priceComponentsQuery = useQuery(
    listPriceComponents,
    { planVersionId: planVersionId ?? '' },
    { enabled: Boolean(planVersionId) }
  )

  // Same query as SubscribablePlanVersionSelect: only active versions can be picked.
  const plansQuery = useQuery(listPlans, {
    filters: { statuses: [PlanStatus.ACTIVE], types: [] },
  })

  const planComponents = priceComponentsQuery.data?.components
  const plans = plansQuery.data?.plans

  return {
    quote,
    planComponents,
    planVersionIsActive: Boolean(plans?.some(p => p.activeVersion?.id === planVersionId)),
    isReady: Boolean(quote && planComponents && plans),
    isError: quoteQuery.isError,
  }
}
