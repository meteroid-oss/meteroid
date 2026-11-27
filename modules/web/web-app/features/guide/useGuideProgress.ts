import { timestampFromDate } from '@bufbuild/protobuf/wkt'
import { useMemo } from 'react'

import { useQuery } from '@/lib/connectrpc'
import { listBillableMetrics } from '@/rpc/api/billablemetrics/v1/billablemetrics-BillableMetricsService_connectquery'
import { listCustomers } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { searchEvents } from '@/rpc/api/events/v1/events-EventsService_connectquery'
import { listInvoices } from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { QuoteStatus } from '@/rpc/api/quotes/v1/models_pb'
import { listQuotes } from '@/rpc/api/quotes/v1/quotes-QuotesService_connectquery'
import { listSubscriptions } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type { GuideSignal } from './guideSteps'

// "Is there at least one?" needs a single row.
const FIRST = { pagination: { perPage: 1, page: 0 } }
// The guide doesn't need to be instant; a minute old is fine and spares the API.
const OPTIONS = { staleTime: 60_000 }

const hasAny = (total: number | bigint | undefined) => Number(total ?? 0) > 0

/** Which guide signals the tenant's data already shows. Queries run only while `enabled`. */
export const useGuideSignals = (enabled: boolean): Record<GuideSignal, boolean> => {
  const options = { ...OPTIONS, enabled }
  // Events have no total: ask for one event since the beginning. Fixed once, so the query key
  // doesn't change on every render.
  const eventsWindow = useMemo(
    () => ({ from: timestampFromDate(new Date(2000, 0, 1)), to: timestampFromDate(new Date()) }),
    []
  )

  const plans = useQuery(listPlans, FIRST, options).data
  const customers = useQuery(listCustomers, FIRST, options).data
  const subscriptions = useQuery(listSubscriptions, FIRST, options).data
  const invoices = useQuery(listInvoices, FIRST, options).data
  const quotes = useQuery(listQuotes, FIRST, options).data
  const accepted = useQuery(listQuotes, { ...FIRST, status: QuoteStatus.ACCEPTED }, options).data
  const metrics = useQuery(listBillableMetrics, FIRST, options).data
  const events = useQuery(searchEvents, { ...eventsWindow, limit: 1, offset: 0 }, options).data

  return {
    plans: hasAny(plans?.paginationMeta?.totalItems),
    customers: hasAny(customers?.paginationMeta?.totalItems),
    subscriptions: hasAny(subscriptions?.paginationMeta?.totalItems),
    invoices: hasAny(invoices?.paginationMeta?.totalItems),
    quotes: hasAny(quotes?.paginationMeta?.totalItems),
    acceptedQuotes: hasAny(accepted?.paginationMeta?.totalItems),
    metrics: hasAny(metrics?.paginationMeta?.totalItems),
    events: (events?.events.length ?? 0) > 0,
  }
}
