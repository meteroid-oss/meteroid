import { LineItemKind } from '@/rpc/api/invoices/v1/models_pb'

import type { PricedFee } from '@/features/subscriptions/subscriptionPrice'
import type { LineItem } from '@/rpc/api/invoices/v1/models_pb'
import type { Wallet } from '@/rpc/api/wallets/v1/models_pb'

/** The customer's wallets that pay this subscription: its currency, or a credit unit its fees use. */
export const subscriptionWallets = (wallets: Wallet[], currency: string, fees: PricedFee[]) => {
  const unitIds = new Set(
    fees.flatMap(f => {
      const unit = f.fee?.creditUnit
      return unit ? [unit.creditUnitId] : []
    })
  )
  return wallets.filter(w =>
    w.creditUnit ? unitIds.has(w.creditUnit.id) : w.currency === currency
  )
}

export const isUsageLine = (line: LineItem) =>
  Boolean(line.metricId) && line.kind === LineItemKind.REGULAR

/**
 * The usage an invoice prices, in the wallet's denomination: credits consumed for a credit-unit
 * wallet, else the usage lines' money.
 */
export const periodUsageFor = (
  wallet: Pick<Wallet, 'creditUnit'>,
  invoice: { lineItems: LineItem[] }
): bigint => {
  const usageLines = invoice.lineItems.filter(isUsageLine)
  if (wallet.creditUnit) {
    const unitId = wallet.creditUnit.id
    return usageLines
      .filter(line => line.creditUnit?.creditUnitId === unitId)
      .reduce((sum, line) => sum + (line.creditUnit?.consumed ?? 0n), 0n)
  }
  return usageLines.filter(line => !line.creditUnit).reduce((sum, line) => sum + line.subtotal, 0n)
}

/** A wallet's credit this subscription can use: its own, and the customer's shared credit. */
export type SubscriptionCreditSplit = {
  own: bigint
  shared: bigint
  /** The label of the subscription's own credit when it is all of one group. */
  ownLabel?: string
  /** Other subscriptions hold credit of their own in the wallet. */
  othersHoldCredit: boolean
}

export const subscriptionCreditSplit = (
  wallet: Pick<Wallet, 'groups'>,
  subscriptionId: string
): SubscriptionCreditSplit => {
  const own = wallet.groups.filter(g => g.subscriptionId === subscriptionId)
  const shared = wallet.groups.filter(g => !g.subscriptionId)
  const labels = new Set(own.map(g => g.label))
  return {
    own: own.reduce((sum, g) => sum + g.available, 0n),
    shared: shared.reduce((sum, g) => sum + g.available, 0n),
    ownLabel: labels.size === 1 ? [...labels][0] : undefined,
    othersHoldCredit: wallet.groups.some(
      g => g.subscriptionId && g.subscriptionId !== subscriptionId && g.available > 0n
    ),
  }
}

/** "Included with Pro $X · Customer-wide credit $Y", or the one kind present; none when it adds nothing. */
export const creditSplitLine = (
  split: SubscriptionCreditSplit,
  format: (minor: bigint) => string
): string | undefined => {
  // The server's group label ("Included with Pro") when the subscription's credit is one group.
  const own = `${split.ownLabel ?? 'Subscription credit'} ${format(split.own)}`
  if (split.own > 0n && split.shared > 0n)
    return `${own} · Customer-wide credit ${format(split.shared)}`
  if (split.own > 0n) return own
  if (split.shared > 0n && split.othersHoldCredit)
    return `Customer-wide credit ${format(split.shared)}`
  return undefined
}
