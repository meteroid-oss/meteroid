import { formatSubscriptionFee } from '@/features/subscriptions/utils/fees'
import { formatCurrencyNoRounding } from '@/utils/numbers'

import type { CreditUnitNames } from '@/features/subscriptions/utils/fees'
import type { SubscriptionFee } from '@/rpc/api/subscriptions/v1/models_pb'

export interface LineFigures {
  /** What one unit costs, or the flat price. */
  unitPrice: string
  /** The amount per period, when the fee states it. */
  amount?: string
  /** The quantity a fixed-quantity fee is sold in. */
  quantity?: number
  /** Tiers, overage or granted credit, for a tooltip. */
  detail?: string
}

/** The figures of a fee, as the quote document formats them: nothing is computed here. */
export function feeFigures(
  fee: SubscriptionFee,
  currency: string,
  creditUnits?: CreditUnitNames
): LineFigures {
  const formatted = formatSubscriptionFee(fee, currency, creditUnits)
  switch (fee.fee.case) {
    case 'rate':
      return { unitPrice: formatted.amount, amount: formatted.amount }
    case 'slot':
      return {
        unitPrice: `${formatCurrencyNoRounding(fee.fee.value.unitRate, currency)} per seat`,
      }
    case 'capacity':
      return { unitPrice: formatted.amount, amount: formatted.amount, detail: formatted.breakdown }
    case 'usage':
      return { unitPrice: formatted.amount, detail: formatted.breakdown }
    case 'oneTime':
    case 'recurring':
      return {
        unitPrice: formatCurrencyNoRounding(fee.fee.value.rate, currency),
        amount: formatted.amount,
        quantity: fee.fee.value.quantity,
      }
    case 'credit':
      return {
        unitPrice: formatCurrencyNoRounding(fee.fee.value.rate, currency),
        amount: formatted.amount,
        quantity: fee.fee.value.quantity,
        detail: [formatted.details, formatted.breakdown].filter(Boolean).join('\n'),
      }
    default:
      return { unitPrice: formatted.amount }
  }
}
