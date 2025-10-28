import { PlainMessage } from "@bufbuild/protobuf"

import { SubscriptionFee } from "@/rpc/api/subscriptions/v1/models_pb"
import { formatCurrencyNoRounding } from "@/utils/numbers"

export const formatSubscriptionFee = (
  fee: PlainMessage<SubscriptionFee> | undefined,
  currency: string
): {
  type: string
  details: string
  amount: string
} => {
  if (!fee || !fee.fee.case) {
    return {
      type: 'N/A',
      details: 'No fee information',
      amount: '-',
    }
  }

  switch (fee.fee.case) {
    case 'rate': {
      return {
        type: 'Rate',
        details: 'Flat rate fee',
        amount: formatCurrencyNoRounding(Number(fee.fee.value.rate), currency),
      }
    }

    case 'oneTime': {
      const oneTimeFee = fee.fee.value
      return {
        type: 'One-time',
        details:
          oneTimeFee.quantity > 1
            ? `${oneTimeFee.quantity}x @ ${oneTimeFee.rate}`
            : 'Single payment',
        amount: formatCurrencyNoRounding(Number(oneTimeFee.total || oneTimeFee.rate), currency),
      }
    }

    case 'recurring': {
      const recurringFee = fee.fee.value
      const billingType = recurringFee.billingType === 0 ? 'in arrears' : 'in advance'
      return {
        type: 'Recurring',
        details:
          recurringFee.quantity > 1
            ? `${recurringFee.quantity}x @ ${recurringFee.rate} (${billingType})`
            : `Recurring ${billingType}`,
        amount: formatCurrencyNoRounding(Number(recurringFee.total || recurringFee.rate), currency),
      }
    }

    case 'capacity': {
      const capacityFee = fee.fee.value
      return {
        type: 'Capacity',
        details: `${capacityFee.included.toString()} included${parseFloat(capacityFee.overageRate) > 0 ? `, then ${formatCurrencyNoRounding(Number(capacityFee.overageRate), currency)} per unit` : ''}`,
        amount: formatCurrencyNoRounding(Number(capacityFee.rate), currency),
      }
    }

    case 'slot': {
      const slotFee = fee.fee.value
      const limits = []

      if (slotFee.minSlots !== undefined) {
        limits.push(`min: ${slotFee.minSlots}`)
      }

      if (slotFee.maxSlots !== undefined) {
        limits.push(`max: ${slotFee.maxSlots}`)
      }

      const limitStr = limits.length > 0 ? ` (${limits.join(', ')})` : ''

      return {
        type: 'Slot',
        details: `${slotFee.initialSlots} ${slotFee.unit}(s)${limitStr}`,
        amount: `${formatCurrencyNoRounding(Number(slotFee.unitRate), currency)} per ${slotFee.unit}`,
      }
    }

    case 'usage': {
      const usageFee = fee.fee.value
      let pricingModel = 'Usage-based'

      // Check for common usage pricing models that might be in the UsageFee
      if ('tiered' in usageFee) {
        pricingModel = 'Tiered'
      } else if ('volume' in usageFee) {
        pricingModel = 'Volume'
      } else if ('package' in usageFee) {
        pricingModel = 'Package'
      }

      return {
        type: 'Usage',
        details: pricingModel,
        amount: 'Variable',
      }
    }

    default:
      return {
        type: 'Unknown',
        details: `Unknown Fee type `,
        amount: '-',
      }
  }
}

/**
 * Formats a subscription fee directly for table display
 *
 * @param fee The subscription fee to format
 * @returns A compact string representation of the fee
 */
export const formatSubscriptionFeeCompact = (
  fee: SubscriptionFee | undefined,
  currency: string
): string => {
  if (!fee || !fee.fee.case) {
    return 'N/A'
  }

  const formatted = formatSubscriptionFee(fee, currency)
  return `${formatted.type}: ${formatted.amount}`
}
