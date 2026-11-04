import { formatWalletAmount } from '@/features/credit/format'
import { toMinor } from '@/features/subscriptions/subscriptionPrice'
import { currencyPrecision } from '@/lib/data/currency-precision'
import { formatCurrency } from '@/lib/utils/numbers'
import { PlanChangePackLine_Kind } from '@/rpc/api/subscriptions/v1/models_pb'

import type { CreditUnitNames } from '@/features/subscriptions/subscriptionPrice'
import type {
  PlanChangePackLine,
  SubscriptionFee_FeeCreditUnit,
} from '@/rpc/api/subscriptions/v1/models_pb'

/** The credit a plan-change pack line grants or removes: "2,500 Halyard credits", or "$275.00 credit". */
export const formatPackLineCredit = (
  line: PlanChangePackLine,
  currency: string,
  unitNames: (unit: SubscriptionFee_FeeCreditUnit) => CreditUnitNames
): string => {
  if (line.creditUnit) {
    const names = unitNames(line.creditUnit)
    return formatWalletAmount(toMinor(line.creditAmount, names.precision), {
      currency: line.creditUnit.code,
      creditUnit: names,
    })
  }
  const minor = toMinor(line.creditAmount, currencyPrecision(currency))
  return `${formatWalletAmount(minor, { currency })} credit`
}

export interface PackLineView {
  label: string
  detail?: string
  value: string
}

const MINUS = '−'
const signedMoney = (amountCents: bigint, currency: string): string => {
  const abs = amountCents < 0n ? -amountCents : amountCents
  return `${amountCents < 0n ? MINUS : '+'}${formatCurrency(abs, currency)}`
}

/** How a plan-change pack line reads: one sign convention, and an included pack as credit, not +$0.00. */
export const packLineView = (
  line: PlanChangePackLine,
  currency: string,
  credit: string
): PackLineView | undefined => {
  switch (line.kind) {
    case PlanChangePackLine_Kind.CHARGE:
      return line.amountCents === 0n
        ? { label: 'Included credit granted', detail: undefined, value: `+${credit}` }
        : {
            label: 'Credit pack billed now',
            detail: `${credit} granted`,
            value: signedMoney(line.amountCents, currency),
          }
    case PlanChangePackLine_Kind.CREDIT_REMOVED:
      return {
        label: 'Unused credit removed',
        detail: line.promotional ? 'Bonus or included credit' : undefined,
        value: `${MINUS}${credit}`,
      }
    case PlanChangePackLine_Kind.FEE_CREDIT:
      return {
        label: 'Unused pack fee credited',
        detail: undefined,
        value: signedMoney(line.amountCents, currency),
      }
    default:
      return undefined
  }
}
