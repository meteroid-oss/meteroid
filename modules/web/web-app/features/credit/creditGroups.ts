import { expires, formatWalletAmount, rollsOver } from '@/features/credit/format'
import { formatDate } from '@/features/credit/labels'
import { CreditGrantKind } from '@/rpc/api/wallets/v1/models_pb'

import type { WalletDenomination } from '@/features/credit/format'
import type { CreditGroup } from '@/rpc/api/wallets/v1/models_pb'

export type CreditGroupRow = {
  key: string
  label: string
  /** The usage filter of the subscription the credit belongs to, e.g. "team = mobile". */
  usageFilter?: string
  amount: string
  scheduled?: string
  paysFor: string
  lapse?: string
  rollover?: string
}

/**
 * A wallet's credit groups worded for display: each with its own amount, expiry and use.
 * With each grant's unbilled usage, a group's amount is what is left after it, as its grants say.
 */
export const creditGroupRows = (
  groups: CreditGroup[],
  denomination: WalletDenomination,
  usageFilters: Map<string, string> = new Map(),
  unbilledByGrant?: Map<string, bigint>
): CreditGroupRow[] =>
  groups.map(group => {
    const amount = (minor: bigint) => formatWalletAmount(minor, denomination)
    // Plan credit renews each period, so its lapse reads as a reset.
    const lapse = !group.nextLapseAt
      ? undefined
      : group.kind === CreditGrantKind.INCLUDED
        ? `Resets ${formatDate(group.nextLapseAt)}`
        : group.nextLapseAmount > 0n && group.nextLapseAmount < group.available
          ? `${amount(group.nextLapseAmount)} ${expires(group.nextLapseAmount, denomination.creditUnit)} ${formatDate(group.nextLapseAt)}`
          : `Expires ${formatDate(group.nextLapseAt)}`
    const rollover = group.rollover
      ? group.rollover.maxAmount !== undefined
        ? `Up to ${amount(group.rollover.maxAmount)} ${rollsOver(group.rollover.maxAmount, denomination.creditUnit)} for ${group.rollover.term}`
        : `All of it rolls over for ${group.rollover.term}`
      : undefined
    return {
      key: `${group.kind}-${group.subscriptionId ?? 'any'}-${group.appliesTo}-${group.productIds.join(',')}`,
      label: group.label,
      usageFilter: group.subscriptionId ? usageFilters.get(group.subscriptionId) : undefined,
      amount: amount(
        group.available -
          (unbilledByGrant
            ? group.creditGrantIds.reduce((sum, id) => sum + (unbilledByGrant.get(id) ?? 0n), 0n)
            : 0n)
      ),
      scheduled: group.upcoming > 0n ? `+ ${amount(group.upcoming)} scheduled` : undefined,
      paysFor: group.paysFor,
      lapse,
      rollover,
    }
  })
