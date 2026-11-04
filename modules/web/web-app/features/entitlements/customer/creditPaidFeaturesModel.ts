import { formatWalletAmount } from '@/features/credit/format'
import { periodUsageFor } from '@/features/subscriptions/subscriptionWallets'
import { CreditExhaustedRule_Kind } from '@/rpc/api/plans/v1/models_pb'
import {
  CreditGrantKind,
  CreditGrantSource,
  CreditGrantStatus,
  UsageAtZero,
} from '@/rpc/api/wallets/v1/models_pb'

import type { CreditPayer } from '@/rpc/api/entitlements/v1/entitlements_pb'
import type { LineItem } from '@/rpc/api/invoices/v1/models_pb'
import type { CreditGroup, Wallet } from '@/rpc/api/wallets/v1/models_pb'

export type ExhaustedRule = 'pause' | 'cap' | 'invoice'

/** The rule in effect for the paying subscriptions: the wallet override, else their plans' rule. */
export const creditExhaustedRule = (
  wallet: Pick<Wallet, 'usageAtZero' | 'planRules'>,
  payers: Pick<CreditPayer, 'subscriptionId'>[]
): ExhaustedRule => {
  if (wallet.usageAtZero === UsageAtZero.ALWAYS_PAUSE) return 'pause'
  if (wallet.usageAtZero === UsageAtZero.NEVER_PAUSE) return 'invoice'
  const kinds = wallet.planRules
    .filter(rule => payers.some(payer => payer.subscriptionId === rule.subscriptionId))
    .map(rule => rule.rule?.kind ?? CreditExhaustedRule_Kind.INVOICE)
  if (kinds.includes(CreditExhaustedRule_Kind.PAUSE)) return 'pause'
  if (kinds.includes(CreditExhaustedRule_Kind.INVOICE_UP_TO)) return 'cap'
  return 'invoice'
}

export const exhaustedRuleLabel: Record<ExhaustedRule, string> = {
  pause: 'Access denied at zero',
  cap: 'Invoiced up to a cap, then access denied',
  invoice: 'Invoiced beyond credit',
}

/** The wallet's credit groups that can pay any of the feature's charges. */
export const payingGroups = (
  groups: CreditGroup[],
  payers: Pick<CreditPayer, 'subscriptionId' | 'productId'>[]
): CreditGroup[] =>
  groups.filter(group =>
    payers.some(
      payer =>
        (!group.subscriptionId || group.subscriptionId === payer.subscriptionId) &&
        (group.productIds.length === 0 ||
          (payer.productId !== undefined && group.productIds.includes(payer.productId)))
    )
  )

export type CreditSource = { label: string; amount: bigint }

const KIND_NAME: Partial<Record<CreditGrantKind, string>> = {
  [CreditGrantKind.PREPAID]: 'Purchased credit',
  [CreditGrantKind.BONUS]: 'Free credit',
  [CreditGrantKind.ACCOUNT_BALANCE]: 'Account balance',
}

/**
 * Where the credit comes from, kind by kind ("Studio pack", "Purchased credit"…), with what each holds.
 * Plan credit carried over from a past period reads apart, as "Rollover".
 */
export const creditSources = (
  groups: CreditGroup[],
  wallet: Pick<Wallet, 'grants'>
): CreditSource[] => {
  const sources: CreditSource[] = []
  const add = (label: string, amount: bigint) => {
    if (amount <= 0n) return
    const source = sources.find(s => s.label === label)
    if (source) source.amount += amount
    else sources.push({ label, amount })
  }
  for (const group of groups) {
    if (group.kind !== CreditGrantKind.INCLUDED) {
      add(KIND_NAME[group.kind] ?? group.label, group.available)
      continue
    }
    const grants = wallet.grants.filter(
      g => group.creditGrantIds.includes(g.id) && g.status === CreditGrantStatus.ACTIVE
    )
    const rolled = grants
      .filter(g => g.source === CreditGrantSource.ROLLOVER)
      .reduce((sum, g) => sum + g.remaining, 0n)
    const plan = grants.find(g => g.subscriptionPlanName)?.subscriptionPlanName
    add(plan ? `${plan} pack` : 'Plan pack', group.available - rolled)
    add('Rollover', rolled)
  }
  return sources
}

/** "Studio pack + purchased credit", or "Multiple sources" past two kinds (a rollover counts with its pack). */
export const sourcesSummary = (sources: CreditSource[]): string | undefined => {
  const kinds = sources.filter(s => s.label !== 'Rollover').map(s => s.label)
  if (kinds.length === 0) return sources.length > 0 ? 'Rollover' : undefined
  if (kinds.length > 2) return 'Multiple sources'
  return kinds.map((kind, i) => (i === 0 ? kind : kind.toLowerCase())).join(' + ')
}

/** The soonest reset of the plan credit paying the feature, which renews with the period. */
export const refreshesAt = (groups: CreditGroup[]) =>
  groups
    .filter(g => g.kind === CreditGrantKind.INCLUDED && g.nextLapseAt)
    .map(g => g.nextLapseAt!)
    .sort((a, b) => Number(a.seconds - b.seconds))[0]

/**
 * The feature's usage this period in the wallet's denomination, as the paying subscriptions'
 * next invoices price its metric. Unknown until each of them is loaded.
 */
export const featurePeriodUsage = (
  wallet: Pick<Wallet, 'creditUnit'>,
  metricId: string,
  invoices: ({ lineItems: LineItem[] } | undefined)[]
): bigint | undefined => {
  if (invoices.length === 0 || invoices.some(invoice => !invoice)) return undefined
  return invoices.reduce(
    (sum, invoice) =>
      sum +
      periodUsageFor(wallet, {
        lineItems: invoice!.lineItems.filter(line => line.metricId === metricId),
      }),
    0n
  )
}

/** "3,900 / 7,620 Halyard credits", or "$39.00 / $76.20". */
export const usageOfCredit = (
  used: bigint,
  credit: bigint,
  wallet: Parameters<typeof formatWalletAmount>[1]
) => `${formatWalletAmount(used, wallet, { bare: true })} / ${formatWalletAmount(credit, wallet)}`
