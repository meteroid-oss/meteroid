import {
  fixedQuantityDetails,
  formatSubscriptionFee,
  slotFeeTotal,
  slotFeeWords,
} from '@/features/subscriptions/utils/fees'
import { formatCurrencyNoRounding } from '@/utils/numbers'

import type { CreditUnitNames } from '@/features/subscriptions/utils/fees'
import type { SubscriptionFee } from '@/rpc/api/subscriptions/v1/models_pb'

export interface LineFigures {
  /** What one unit costs, or the flat price. */
  unitPrice: string
  /** `unitPrice` is the price in words: a model prices the count, no single rate does. */
  inWords?: boolean
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
    case 'slot': {
      // Seats priced by a model have no single rate: the model in words, its total at the count.
      const slot = fee.fee.value
      return {
        unitPrice: slotFeeWords(slot, currency),
        inWords: Boolean(slot.model?.model.case),
        amount: formatCurrencyNoRounding(slotFeeTotal(slot, slot.initialSlots).toString(), currency),
        quantity: slot.initialSlots,
      }
    }
    case 'capacity':
      return { unitPrice: formatted.amount, amount: formatted.amount, detail: formatted.breakdown }
    case 'usage':
      return { unitPrice: formatted.amount, detail: formatted.breakdown }
    case 'oneTime':
    case 'recurring': {
      // A quantity priced by a model reads as the model, not as its ignored rate.
      const priced = fee.fee.value.model?.model.case
      return {
        unitPrice: priced
          ? (fixedQuantityDetails(fee.fee.value, currency) ?? formatted.amount)
          : formatCurrencyNoRounding(fee.fee.value.rate, currency),
        inWords: Boolean(priced),
        amount: formatted.amount,
        quantity: fee.fee.value.quantity,
      }
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
