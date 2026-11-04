import { createQueryOptions, useTransport } from '@connectrpc/connect-query'
import { useQueries } from '@tanstack/react-query'

import { subscriptionWallets } from '@/features/subscriptions/subscriptionWallets'
import { useQuery } from '@/lib/connectrpc'
import { SubscriptionStatus } from '@/rpc/api/subscriptions/v1/models_pb'
import {
  getSubscriptionDetails,
  listSubscriptions,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type { Wallet } from '@/rpc/api/wallets/v1/models_pb'

export type FundingSubscription = { id: string; start: string; end: string }

export type WalletPeriod = {
  start: string
  end: string
  subscriptionId: string
  /** Every live subscription drawing on the wallet, the soonest-ending first. */
  subscriptions: FundingSubscription[]
}

/**
 * `too-many` and `error`: which subscriptions fund which wallet is not fully known, so no wallet
 * may show a sum of its subscriptions' usage.
 */
export type WalletPeriodsStatus = 'loading' | 'ready' | 'too-many' | 'error'

// Past this many live subscriptions the card states that usage is not shown, never a partial sum.
export const MAX_FUNDING_SUBSCRIPTIONS = 20

/**
 * The current period of the live subscriptions each wallet funds (its currency, or a credit unit
 * their fees use), soonest-ending first.
 */
export const useWalletPeriods = (
  customerId: string,
  wallets: Wallet[],
  enabled: boolean
): { periods: Map<string, WalletPeriod>; status: WalletPeriodsStatus } => {
  const transport = useTransport()
  const live = useQuery(
    listSubscriptions,
    {
      customerId,
      status: [SubscriptionStatus.ACTIVE, SubscriptionStatus.TRIALING],
      pagination: { perPage: MAX_FUNDING_SUBSCRIPTIONS, page: 0 },
    },
    { enabled }
  )
  const listed = live.data?.subscriptions ?? []
  const subscriptions = listed
    .filter(s => s.currentPeriodStart && s.currentPeriodEnd)
    .sort((a, b) => (a.currentPeriodEnd ?? '').localeCompare(b.currentPeriodEnd ?? ''))
  const details = useQueries({
    queries: subscriptions.map(s => ({
      ...createQueryOptions(getSubscriptionDetails, { subscriptionId: s.id }, { transport }),
      enabled,
    })),
  })

  const status: WalletPeriodsStatus =
    live.isError || details.some(result => result.isError)
      ? 'error'
      : live.isLoading || details.some(result => result.isLoading)
        ? 'loading'
        : (live.data?.paginationMeta?.totalItems ?? listed.length) > listed.length
          ? 'too-many'
          : 'ready'

  const periods = new Map<string, WalletPeriod>()
  if (status !== 'ready') return { periods, status }
  details.forEach(result => {
    const data = result.data
    const subscription = data?.subscription
    if (!data || !subscription?.currentPeriodStart || !subscription.currentPeriodEnd) return
    const fees = [
      ...data.priceComponents.map(c => ({ fee: c.fee, period: c.period })),
      ...data.addOns.map(a => ({ fee: a.fee, period: a.period })),
    ]
    const funding = {
      id: subscription.id,
      start: subscription.currentPeriodStart,
      end: subscription.currentPeriodEnd,
    }
    for (const wallet of subscriptionWallets(wallets, subscription.currency, fees)) {
      const period = periods.get(wallet.id)
      if (period) {
        period.subscriptions.push(funding)
      } else {
        periods.set(wallet.id, {
          start: funding.start,
          end: funding.end,
          subscriptionId: funding.id,
          subscriptions: [funding],
        })
      }
    }
  })
  return { periods, status }
}
