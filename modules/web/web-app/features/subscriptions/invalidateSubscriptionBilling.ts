import { createConnectQueryKey } from '@connectrpc/connect-query'

import { listInvoices } from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import {
  getSlotsValue,
  getSubscriptionDetails,
  getSubscriptionMinimumProgress,
  getSubscriptionReceivables,
  getUpcomingInvoice,
  listSlotTransactions,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { listCustomerWallets } from '@/rpc/api/wallets/v1/wallets-WalletsService_connectquery'

import type { QueryClient } from '@tanstack/react-query'

/**
 * After a change to what a subscription bills (activation, cancellation and its undo, a plan
 * change, an amendment, seats): its details, next invoice, invoices, receivables and credit.
 */
export const invalidateSubscriptionBilling = (queryClient: QueryClient, subscriptionId: string) => {
  const input = { subscriptionId }
  const keys = [
    createConnectQueryKey({ schema: getSubscriptionDetails, input, cardinality: 'finite' }),
    createConnectQueryKey({ schema: getUpcomingInvoice, input, cardinality: 'finite' }),
    createConnectQueryKey({ schema: getSubscriptionReceivables, input, cardinality: 'finite' }),
    createConnectQueryKey({ schema: getSubscriptionMinimumProgress, input, cardinality: 'finite' }),
    // Keyed by more than the subscription: every list is refreshed.
    createConnectQueryKey({ schema: listInvoices, cardinality: undefined }),
    createConnectQueryKey({ schema: listCustomerWallets, cardinality: undefined }),
    createConnectQueryKey({ schema: listSlotTransactions, cardinality: undefined }),
    createConnectQueryKey({ schema: getSlotsValue, cardinality: undefined }),
  ]
  return Promise.all(keys.map(queryKey => queryClient.invalidateQueries({ queryKey })))
}
