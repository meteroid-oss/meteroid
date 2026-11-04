import { timestampDate } from '@bufbuild/protobuf/wkt'
import Decimal from 'decimal.js'

import { averageUsage } from '@/features/subscriptions/subscriptionOverview'
import { packCredits } from '@/features/subscriptions/subscriptionPrice'
import { SubscriptionFeeBillingPeriod } from '@/rpc/api/subscriptions/v1/models_pb'
import { CreditGrantKind } from '@/rpc/api/wallets/v1/models_pb'

import type { PricedFee } from '@/features/subscriptions/subscriptionPrice'
import type { CreditGrant } from '@/rpc/api/wallets/v1/models_pb'

/**
 * The wallet's average usage per period: the sum of each subscription's average over its own
 * recent invoiced periods (newest first), with the most periods any of them averaged.
 */
export const walletAverageUsage = (
  perSubscription: bigint[][]
): { average: bigint; periods: number } | undefined => {
  const averaged = perSubscription.filter(usages => usages.length > 0)
  if (averaged.length === 0) return undefined
  return {
    average: averaged.reduce((sum, usages) => sum + (averageUsage(usages) ?? 0n), 0n),
    periods: Math.max(...averaged.map(usages => usages.length)),
  }
}

/** The credit the subscriptions' packs grant in `key` each month; none unless every pack is monthly. */
export const walletMonthlyIncluded = (
  subscriptions: { fees: PricedFee[]; currency: string }[],
  key: string
): Decimal | undefined => {
  const packs = subscriptions
    .flatMap(s => packCredits(s.fees, s.currency))
    .filter(p => p.key === key && !p.once)
  if (packs.length === 0) return undefined
  if (packs.some(p => p.period !== SubscriptionFeeBillingPeriod.MONTHLY)) return undefined
  return packs.reduce((sum, p) => sum.add(p.total), new Decimal(0))
}

export type PeriodCredit = { plan: bigint; added: bigint }

/**
 * The credit that took effect since the period start: the plan's (included, rolled over) and what
 * was added beyond it (bought or given). With `subscriptionId`, that subscription's and the
 * customer-wide credit only.
 */
export const periodCreditGranted = (
  grants: Pick<CreditGrant, 'kind' | 'amount' | 'effectiveAt' | 'subscriptionId'>[],
  since: string,
  now: Date,
  subscriptionId?: string
): PeriodCredit => {
  const from = Date.parse(since)
  let plan = 0n
  let added = 0n
  for (const grant of grants) {
    if (subscriptionId && grant.subscriptionId && grant.subscriptionId !== subscriptionId) continue
    if (!grant.effectiveAt) continue
    const at = timestampDate(grant.effectiveAt).getTime()
    if (at < from || at > now.getTime()) continue
    if (grant.kind === CreditGrantKind.INCLUDED) plan += grant.amount
    else if (grant.kind === CreditGrantKind.PREPAID || grant.kind === CreditGrantKind.BONUS) {
      added += grant.amount
    }
  }
  return { plan, added }
}

/** The share of the period's credit used, "76%"; none without plan credit for the period. */
export const periodUtilization = (
  used: bigint,
  credit: PeriodCredit
): { total: bigint; label: string } | undefined => {
  if (credit.plan <= 0n) return undefined
  const total = credit.plan + credit.added
  const pct = Math.round(Number((used * 1000n) / total) / 10)
  return { total, label: used > 0n && pct === 0 ? '<1%' : `${pct}%` }
}
