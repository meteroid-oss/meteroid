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
  breakdown?: string // Optional detailed breakdown for complex pricing
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

      switch (usageFee.model?.case) {
        case 'perUnit': {
          const perUnitPrice = usageFee.model.value
          return {
            type: 'Usage',
            details: 'Per unit',
            amount: `${formatCurrencyNoRounding(Number(perUnitPrice), currency)}/unit`,
          }
        }

        case 'tiered': {
          const tiered = usageFee.model.value
          const rows = tiered.rows || []
          if (rows.length === 0) {
            return { type: 'Usage', details: 'Tiered', amount: '-' }
          }
          // Summary: show price range
          const prices = rows.map(r => Number(r.unitPrice))
          const minPrice = Math.min(...prices)
          const maxPrice = Math.max(...prices)
          const summary = minPrice === maxPrice
            ? `${formatCurrencyNoRounding(minPrice, currency)}/unit`
            : `${formatCurrencyNoRounding(minPrice, currency)} - ${formatCurrencyNoRounding(maxPrice, currency)}/unit`
          // Breakdown: show all tiers
          const breakdown = rows.map((row, idx) => {
            const nextRow = rows[idx + 1]
            const rangeStart = Number(row.firstUnit)
            const rangeEnd = nextRow ? Number(nextRow.firstUnit) - 1 : '∞'
            const price = formatCurrencyNoRounding(Number(row.unitPrice), currency)
            return `${rangeStart}-${rangeEnd}: ${price}`
          }).join('\n')
          return {
            type: 'Usage',
            details: 'Tiered',
            amount: summary,
            breakdown,
          }
        }

        case 'volume': {
          const volume = usageFee.model.value
          const rows = volume.rows || []
          if (rows.length === 0) {
            return { type: 'Usage', details: 'Volume', amount: '-' }
          }
          // Summary: show price range
          const prices = rows.map(r => Number(r.unitPrice))
          const minPrice = Math.min(...prices)
          const maxPrice = Math.max(...prices)
          const summary = minPrice === maxPrice
            ? `${formatCurrencyNoRounding(minPrice, currency)}/unit`
            : `${formatCurrencyNoRounding(minPrice, currency)} - ${formatCurrencyNoRounding(maxPrice, currency)}/unit`
          // Breakdown: show all tiers
          const breakdown = rows.map((row, idx) => {
            const nextRow = rows[idx + 1]
            const rangeStart = Number(row.firstUnit)
            const rangeEnd = nextRow ? Number(nextRow.firstUnit) - 1 : '∞'
            const price = formatCurrencyNoRounding(Number(row.unitPrice), currency)
            return `${rangeStart}-${rangeEnd}: ${price}`
          }).join('\n')
          return {
            type: 'Usage',
            details: 'Volume',
            amount: summary,
            breakdown,
          }
        }

        case 'package': {
          const pkg = usageFee.model.value
          return {
            type: 'Usage',
            details: 'Package',
            amount: `${formatCurrencyNoRounding(Number(pkg.packagePrice), currency)} / ${pkg.blockSize} units`,
          }
        }

        case 'matrix': {
          const matrix = usageFee.model.value
          const rows = matrix.rows || []
          if (rows.length === 0) {
            return { type: 'Usage', details: 'Matrix', amount: '-' }
          }
          // Summary: show price range
          const prices = rows.map(r => Number(r.perUnitPrice))
          const minPrice = Math.min(...prices)
          const maxPrice = Math.max(...prices)
          const summary = minPrice === maxPrice
            ? `${formatCurrencyNoRounding(minPrice, currency)}/unit`
            : `${formatCurrencyNoRounding(minPrice, currency)} - ${formatCurrencyNoRounding(maxPrice, currency)}/unit`
          // Breakdown: show all matrix price points
          const breakdown = rows.map(row => {
            const dims = [row.dimension1]
            if (row.dimension2) dims.push(row.dimension2)
            const dimStr = dims.map(d => `${d?.key}=${d?.value}`).join(', ')
            const price = formatCurrencyNoRounding(Number(row.perUnitPrice), currency)
            return `${dimStr}: ${price}`
          }).join('\n')
          return {
            type: 'Usage',
            details: 'Matrix',
            amount: summary,
            breakdown,
          }
        }

        default:
          return {
            type: 'Usage',
            details: 'Usage-based',
            amount: 'Variable',
          }
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

  console.log('fee', fee)

  if (!fee || !fee.fee.case) {
    return 'N/A'
  }

  const formatted = formatSubscriptionFee(fee, currency)
  return `${formatted.type}: ${formatted.amount}`
}
