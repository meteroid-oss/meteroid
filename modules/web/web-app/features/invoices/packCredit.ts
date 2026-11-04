import { formatWalletAmount } from '@/features/credit/format'
import { LineItemKind } from '@/rpc/api/invoices/v1/models_pb'

import type { CreditUnitNames } from '@/features/subscriptions/subscriptionPrice'
import type { LineItem } from '@/rpc/api/invoices/v1/models_pb'

/**
 * The credit a credit line grants, as the subscription page states it: "Adds 500 fast requests",
 * or "Adds €50.00 of credit · €40.00 paid + €10.00 bonus credit" for a money pack paid below its
 * credit, priced so or discounted: the ledger mints the paid share and bonus credit for the rest.
 * `units` resolves a unit's names by id; without them its code names it.
 */
export const packCreditText = (
  line: Pick<LineItem, 'kind' | 'creditGrantAmount' | 'creditGrantUnit' | 'taxableAmount'>,
  currency: string,
  units: Record<string, CreditUnitNames>,
  // A credit note's lines are stated negative and take the credit back.
  verb: 'Adds' | 'Reverses' = 'Adds'
): string | undefined => {
  const abs = (value: bigint) => (value < 0n ? -value : value)
  const face = line.creditGrantAmount === undefined ? undefined : abs(line.creditGrantAmount)
  if (line.kind !== LineItemKind.CREDIT || face === undefined || face === 0n) return undefined
  const paid = abs(line.taxableAmount)
  const unit = line.creditGrantUnit
  if (unit) {
    const names = units[unit.creditUnitId] ?? {
      name: unit.code,
      pluralName: unit.code,
      precision: unit.precision,
    }
    return `${verb} ${formatWalletAmount(face, { currency: unit.code, creditUnit: names })}`
  }
  const money = (minor: bigint) => formatWalletAmount(minor, { currency })
  const bonus =
    paid > 0n && paid < face
      ? ` · ${money(paid)} paid + ${money(face - paid)} bonus credit`
      : ''
  return `${verb} ${money(face)} of credit${bonus}`
}
