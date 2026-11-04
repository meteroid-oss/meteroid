import { timestampDate } from '@bufbuild/protobuf/wkt'
import Decimal from 'decimal.js'

import { formatWalletAmount } from '@/features/credit/format'
import { grantSourceLabel } from '@/features/credit/labels'
import { daysPastDue } from '@/features/invoices/invoiceState'
import { toMinor } from '@/features/subscriptions/subscriptionPrice'
import { formatShortDay } from '@/features/subscriptions/utils/dates'
import { currencyPrecision } from '@/lib/data/currency-precision'
import { parseUtc } from '@/lib/utils/date'
import { formatCurrency } from '@/lib/utils/numbers'
import { InvoicePaymentStatus, InvoiceStatus } from '@/rpc/api/invoices/v1/models_pb'
import { SubscriptionStatus } from '@/rpc/api/subscriptions/v1/models_pb'
import { UpcomingInvoiceChange } from '@/rpc/api/subscriptions/v1/subscriptions_pb'
import {
  CreditGrantSource,
  CreditGrantStatus,
  CreditGrantType,
  WalletStatus,
} from '@/rpc/api/wallets/v1/models_pb'

import type { WalletDenomination } from '@/features/credit/format'
import type { PricedFee } from '@/features/subscriptions/subscriptionPrice'
import type { Invoice } from '@/rpc/api/invoices/v1/models_pb'
import type {
  PendingInvoiceCredit,
  SubscriptionReceivables,
} from '@/rpc/api/subscriptions/v1/subscriptions_pb'
import type { CreditGrant, Wallet } from '@/rpc/api/wallets/v1/models_pb'

const PACK_SOURCES = [
  CreditGrantSource.PURCHASE,
  CreditGrantSource.INCLUDED,
  CreditGrantSource.ROLLOVER,
]

export type Tone = 'muted' | 'warning' | 'destructive'
export type Tagged = { text: string; tone: Tone }

/** The unit once more after the row already names it: the symbol, else the last word ("credits"). */
export const unitShortName = (unit: { pluralName: string; symbol?: string }) => {
  if (unit.symbol) return unit.symbol
  const words = unit.pluralName.trim().split(/\s+/)
  return words.length > 1 ? words[words.length - 1].toLowerCase() : unit.pluralName
}

/** "$400.00", or "3,720 credits" for a credit-unit wallet whose row title names the unit. */
export const formatBalance = (
  minor: bigint,
  denomination: WalletDenomination & { creditUnit?: { symbol?: string } }
) => {
  const unit = denomination.creditUnit
  if (!unit) return formatWalletAmount(minor, denomination)
  return `${formatWalletAmount(minor, denomination, { bare: true })} ${unitShortName(unit)}`
}

/** "550.00": an amount whose currency or unit the row already names. */
export const formatBareAmount = (minor: bigint, denomination: WalletDenomination) => {
  if (denomination.creditUnit) return formatWalletAmount(minor, denomination, { bare: true })
  const precision = currencyPrecision(denomination.currency)
  const sign = minor < 0n ? '−' : ''
  const absolute = minor < 0n ? -minor : minor
  return `${sign}${new Intl.NumberFormat('en-US', {
    minimumFractionDigits: precision,
    maximumFractionDigits: precision,
  }).format(new Decimal(absolute.toString()).div(new Decimal(10).pow(precision)).toNumber())}`
}

/** "≈ $420", or "≈ 4,940" for credits: rounded to a whole unit, the unit named elsewhere. */
export const formatApproxBare = (minor: bigint, denomination: WalletDenomination) => {
  const precision = denomination.creditUnit?.precision ?? currencyPrecision(denomination.currency)
  const scale = 10n ** BigInt(precision)
  const rounded = ((minor + scale / 2n) / scale) * scale
  return denomination.creditUnit
    ? `≈ ${formatWalletAmount(rounded, denomination, { bare: true })}`
    : `≈ ${formatCurrency(rounded, denomination.currency, { trimWholeAmounts: true })}`
}

/** "since 11:20 today", "since Oct 7, 11:20". */
export const sinceLabel = (at: Date, now: Date) => {
  const time = at.toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit', hour12: false })
  const sameDay = at.toDateString() === now.toDateString()
  if (sameDay) return `since ${time} today`
  const day = at.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })
  return `since ${day}, ${time}`
}

/** No other subscription holds credit of its own in the wallet. */
export const servesOnly = (wallet: Pick<Wallet, 'groups'>, subscriptionId: string) =>
  !wallet.groups.some(
    g =>
      g.subscriptionId &&
      g.subscriptionId !== subscriptionId &&
      (g.available > 0n || g.upcoming > 0n)
  )

/** The credit on account the subscription can use: its own and the customer-wide credit. */
export const usableCredit = (wallet: Pick<Wallet, 'groups'>, subscriptionId: string) =>
  wallet.groups
    .filter(g => !g.subscriptionId || g.subscriptionId === subscriptionId)
    .reduce((sum, g) => sum + g.available, 0n)

/**
 * The wallet's status badge on a subscription's page, from the ledger: the customer-level low or
 * depleted status shows only when the wallet holds no other subscription's credit.
 */
export const ledgerStatus = (
  wallet: Pick<Wallet, 'groups' | 'status'>,
  subscriptionId: string
): WalletStatus =>
  servesOnly(wallet, subscriptionId) &&
  (wallet.status === WalletStatus.LOW || wallet.status === WalletStatus.DEPLETED)
    ? wallet.status
    : WalletStatus.UNSPECIFIED

const grantExpiry = (grant: CreditGrant) =>
  grant.expiresAt ? timestampDate(grant.expiresAt) : undefined

const spendable = (grant: CreditGrant) =>
  grant.status === CreditGrantStatus.ACTIVE && grant.remaining > 0n

export const grantLabel = (grant: CreditGrant) => grant.reason || grantSourceLabel[grant.source]

export type ExpiryContext = {
  /** Credit the next period's pack grants this wallet, in its minor units. */
  refill?: bigint
  /** Included credit carried over at expiry: up to `max` minor units, all of it without one. */
  rollover?: { max?: bigint }
}

/** At most one muted note: credit expiring by the period end, else credit scheduled to arrive. */
export const creditNote = (
  wallet: Wallet,
  periodEnd: string | undefined,
  { refill, rollover }: ExpiryContext
): string | undefined => {
  const amount = (minor: bigint) => formatWalletAmount(minor, wallet)
  const horizon = periodEnd ? new Date(periodEnd).getTime() : undefined
  const expiring = wallet.grants.filter(g => {
    const at = grantExpiry(g)
    return spendable(g) && at && horizon !== undefined && at.getTime() <= horizon
  })
  if (expiring.length > 0) {
    const first = Math.min(...expiring.map(g => (grantExpiry(g) as Date).getTime()))
    const day = formatShortDay(new Date(first).toISOString())
    // Only credit the plan's packs funded resets or rolls over with them.
    const fromPacks = expiring.every(g => PACK_SOURCES.includes(g.source))
    if (fromPacks && rollover) {
      return rollover.max === undefined
        ? `Included credits expire ${day} · unused credits roll over`
        : `Included credits expire ${day} · up to ${formatWalletAmount(rollover.max, wallet, { bare: true })} roll over`
    }
    if (fromPacks && refill !== undefined) {
      return `Unused credit expires ${day} · resets to ${amount(refill)}`
    }
    // The soonest expiry, with only what expires then.
    const soonest = expiring.filter(g => (grantExpiry(g) as Date).getTime() === first)
    const total = soonest.reduce((sum, g) => sum + g.remaining, 0n)
    return soonest.every(g => g.type === CreditGrantType.PROMOTIONAL)
      ? `${amount(total)} free credit expires ${day}`
      : `${amount(total)} of credit expires ${day}`
  }
  const scheduled = wallet.grants.find(g => g.status === CreditGrantStatus.PENDING)
  if (scheduled?.effectiveAt) {
    return `${amount(scheduled.amount)} added ${formatShortDay(timestampDate(scheduled.effectiveAt).toISOString())}`
  }
  return undefined
}

/** Nothing moving: no unbilled usage, nothing to flag, nothing expiring before the period end. */
export const isIdleWallet = (wallet: Wallet, periodUsage: bigint | undefined, note?: string) => {
  const live = wallet.live
  if (live?.stale) return false
  if (live && (live.pendingUsage > 0n || live.uncoveredUsage > 0n)) return false
  if (periodUsage !== undefined && periodUsage > 0n) return false
  if (wallet.status !== WalletStatus.HEALTHY && wallet.status !== WalletStatus.UNSPECIFIED) {
    return false
  }
  return !note
}

/** "Service credit · expires Jan 1, 2027", from the grant spent first. */
export const idleWalletDetail = (wallet: Wallet) => {
  const grants = wallet.grants.filter(spendable)
  if (grants.length === 0) return undefined
  if (grants.length > 1) return `${grants.length} credit grants`
  const at = grantExpiry(grants[0])
  return [grantLabel(grants[0]), at && `expires ${formatShortDay(at.toISOString())}`]
    .filter(Boolean)
    .join(' · ')
}

/**
 * The rollover of the included packs funding `key` (credit unit id or currency), scaled like the
 * credit they grant: per pack, times its quantity. Uncapped when any of them is.
 */
export const packRollover = (
  fees: PricedFee[],
  currency: string,
  key: string,
  precision: number
): { max?: bigint } | undefined => {
  let max = 0n
  let found = false
  for (const priced of fees) {
    const fee = priced.fee
    if (fee?.fee.case !== 'credit' || !fee.fee.value.rollover) continue
    if ((fee.creditUnit?.creditUnitId ?? currency) !== key) continue
    found = true
    const cap = fee.fee.value.rollover.maxAmount
    if (cap === undefined) return {}
    try {
      const packs = new Decimal(fee.fee.value.quantity || 1).mul(priced.quantity ?? 1)
      max += toMinor(new Decimal(cap).mul(packs), precision)
    } catch {
      return undefined
    }
  }
  return found ? { max } : undefined
}

/**
 * The day the next invoice is issued and dated, after the entity's grace period: finalization
 * re-dates it to that day, and its due date counts the net terms from there.
 */
export const issueDay = (invoice: { chargeDate: string; invoiceDate: string }) =>
  invoice.chargeDate || invoice.invoiceDate

/** What the next invoice leaves to pay once credit is drawn; "≈" when that credit is shared. */
export const nextInvoiceAmount = (
  invoice: { amountDue: bigint; creditEstimateShared: boolean },
  money: (minor: bigint) => string
) => `${invoice.creditEstimateShared ? '≈ ' : ''}${money(invoice.amountDue)}`

/** The next invoice's title, and what it bills after: an ending's final invoice, or a scheduled change. */
export const nextInvoiceTitle = (
  invoice: { change: UpcomingInvoiceChange; newPlanName?: string },
  trialing: boolean
): { title: string; after?: string } => {
  switch (invoice.change) {
    case UpcomingInvoiceChange.ENDS:
      return { title: 'Final invoice' }
    case UpcomingInvoiceChange.PAUSE:
      return { title: 'Last invoice before the pause' }
    case UpcomingInvoiceChange.PLAN_CHANGE:
      return {
        title: 'Next invoice',
        after: invoice.newPlanName
          ? `after the change to ${invoice.newPlanName}`
          : 'after the plan change',
      }
    case UpcomingInvoiceChange.AMENDMENT:
      return { title: 'Next invoice', after: 'after the scheduled changes' }
    default:
      return { title: trialing ? 'First invoice' : 'Next invoice' }
  }
}

export const plural = (count: number, word: string) => `${count} ${count === 1 ? word : `${word}s`}`

/** The header's Outstanding fact; undefined when nothing is owed. */
export const outstandingFact = (
  receivables: SubscriptionReceivables | undefined
): Tagged | undefined => {
  if (!receivables || receivables.openAmount <= 0n) return undefined
  const money = (minor: bigint) => formatCurrency(minor, receivables.currency)
  if (receivables.overdueAmount > 0n) {
    return { text: `${money(receivables.overdueAmount)} overdue`, tone: 'destructive' }
  }
  return {
    text: receivables.nextDueAt
      ? `${money(receivables.openAmount)} · due ${formatShortDay(parseUtc(receivables.nextDueAt).toISOString())}`
      : `${money(receivables.openAmount)} open`,
    tone: 'muted',
  }
}

/** The Outstanding fact's tooltip once something is overdue: which invoices, the oldest's failure and retry. */
export const overdueDetails = (
  receivables: SubscriptionReceivables | undefined,
  now: Date
): string[] => {
  const oldest = receivables?.oldestOverdue
  if (!receivables || !oldest || receivables.overdueAmount <= 0n) return []
  const late = plural(daysPastDue(parseUtc(oldest.dueAt), now), 'day')
  const failedAt = oldest.lastFailureAt ? parseUtc(oldest.lastFailureAt) : undefined
  const retryAt = oldest.nextRetryAt ? parseUtc(oldest.nextRetryAt) : undefined
  const reason = oldest.lastFailureReason ? `: ${oldest.lastFailureReason.replace(/_/g, ' ')}` : ''
  return [
    receivables.overdueCount > 1
      ? `${receivables.overdueCount} invoices overdue · oldest ${oldest.invoiceNumber}, ${late} late.`
      : `${oldest.invoiceNumber} is ${late} late.`,
    failedAt && `Last payment attempt failed ${formatShortDay(failedAt.toISOString())}${reason}.`,
    retryAt && `Next retry ${formatShortDay(retryAt.toISOString())}.`,
  ].filter((line): line is string => Boolean(line))
}

export type PrimaryAction = 'checkout' | 'add-credit' | 'change-plan' | 'amend' | undefined

export const primaryAction = ({
  status,
  pendingCheckout,
  hasWallet,
  isFreePlan,
  hasMinimum,
}: {
  status: SubscriptionStatus
  pendingCheckout: boolean
  hasWallet: boolean
  isFreePlan: boolean
  hasMinimum: boolean
}): PrimaryAction => {
  if (
    status === SubscriptionStatus.CANCELED ||
    status === SubscriptionStatus.ENDED ||
    status === SubscriptionStatus.ABORTED
  ) {
    return undefined
  }
  if (pendingCheckout && status === SubscriptionStatus.PENDING) return 'checkout'
  const live = status === SubscriptionStatus.ACTIVE || status === SubscriptionStatus.TRIALING
  if (hasWallet) return 'add-credit'
  if (live && isFreePlan) return 'change-plan'
  if (live && hasMinimum) return 'amend'
  return undefined
}

/** Usage per period over the latest complete periods, as a rounded average in minor units. */
export const averageUsage = (perPeriod: bigint[]) =>
  perPeriod.length === 0
    ? undefined
    : toMinor(
        perPeriod.reduce((sum, v) => sum.add(v.toString()), new Decimal(0)).div(perPeriod.length),
        0
      )

/** A hard limit's access check denies at its limit plus its grace, as the backend computes it. */
export const limitRefused = (used: string, limit: string, gracePct?: number) => {
  try {
    const ceiling = new Decimal(limit).mul(new Decimal(100).add(gracePct ?? 0)).div(100)
    return new Decimal(used).gte(ceiling)
  } catch {
    return false
  }
}

/**
 * A discounted pack funds its paid price as paid credit and the rest as bonus credit: the cost
 * basis is the line after discounts, before tax, as minting reads it.
 */
export const packSplit = (face: bigint, taxable: bigint) =>
  taxable > 0n && taxable < face ? { paid: taxable, bonus: face - taxable } : undefined

/** What an invoice leaves once credit paid its share: charged when paid, else still due. */
export const invoiceAfterCredit = (
  invoice: Invoice
): { kind: 'none' } | { kind: 'draft' | 'settled' | 'due'; amount: bigint } => {
  if (invoice.status === InvoiceStatus.VOID || invoice.consolidatedIntoInvoiceId) {
    return { kind: 'none' }
  }
  if (invoice.status === InvoiceStatus.DRAFT) return { kind: 'draft', amount: invoice.amountDue }
  if (invoice.paymentStatus === InvoicePaymentStatus.PAID) {
    return { kind: 'settled', amount: invoice.total - invoice.appliedCredits }
  }
  return { kind: 'due', amount: invoice.amountDue }
}

export type WalletlessCredit = {
  /** Credit unit id, or the currency. */
  key: string
  creditUnitId?: string
  /** What the next invoice's pack adds, in the denomination's minor units. */
  nextAdds?: bigint
  /** Credit bought on unpaid invoices, funded once they are paid. */
  pending: PendingInvoiceCredit[]
}

/** Credit this subscription brings that no wallet holds yet: a pack to come or one awaiting payment. */
export const walletlessCredit = (
  packs: { key: string; creditUnitId?: string; total: bigint }[],
  pendingCredit: PendingInvoiceCredit[],
  wallets: Pick<Wallet, 'currency' | 'creditUnit'>[]
): WalletlessCredit[] => {
  const held = new Set(wallets.map(w => w.creditUnit?.id ?? w.currency))
  const rows = new Map<string, WalletlessCredit>()
  const row = (key: string, creditUnitId?: string) => {
    const existing = rows.get(key) ?? { key, creditUnitId, pending: [] }
    rows.set(key, existing)
    return existing
  }
  for (const pack of packs) {
    if (held.has(pack.key) || pack.total === 0n) continue
    const r = row(pack.key, pack.creditUnitId)
    r.nextAdds = (r.nextAdds ?? 0n) + pack.total
  }
  for (const credit of pendingCredit) {
    const key = credit.creditUnitId ?? credit.currency
    if (held.has(key)) continue
    row(key, credit.creditUnitId).pending.push(credit)
  }
  return [...rows.values()]
}
