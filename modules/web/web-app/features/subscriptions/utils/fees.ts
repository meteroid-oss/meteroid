
import { CreditPackExpiry_Unit } from "@/rpc/api/prices/v1/models_pb"
import {
  SubscriptionFee,
  SubscriptionFeeBillingPeriod,
  SubscriptionFee_FeeCreditUnit,
} from "@/rpc/api/subscriptions/v1/models_pb"
import { formatCurrencyNoRounding, formatUsage } from "@/utils/numbers"

import type { CreditUnit } from "@/rpc/api/creditunits/v1/models_pb"

/** The period a fee bills on: a credit granted once at start repeats on none. */
export const feeBillingPeriod = (
  period: SubscriptionFeeBillingPeriod,
  fee: Pick<SubscriptionFee, 'fee'> | undefined
) =>
  fee?.fee.case === 'credit' && fee.fee.value.once ? SubscriptionFeeBillingPeriod.ONE_TIME : period

export const formatSubscriptionFeeBillingPeriod = (
  period: SubscriptionFeeBillingPeriod
): string => {
  switch (period) {
    case SubscriptionFeeBillingPeriod.ONE_TIME:
      return 'One Time'
    case SubscriptionFeeBillingPeriod.MONTHLY:
      return 'Monthly'
    case SubscriptionFeeBillingPeriod.QUARTERLY:
      return 'Quarterly'
    case SubscriptionFeeBillingPeriod.SEMIANNUAL:
      return 'Semiannual'
    case SubscriptionFeeBillingPeriod.YEARLY:
      return 'Yearly'
    default:
      return 'Unknown'
  }
}

export type CreditUnitNames = readonly Pick<CreditUnit, 'id' | 'name' | 'pluralName'>[]

// The unit's code is internal: name it from the tenant's units or the names the fee carries
// (portal checkout), generically until either is known.
const unitNamer = (unit: SubscriptionFee_FeeCreditUnit, units: CreditUnitNames | undefined) => {
  const named = units?.find(candidate => candidate.id === unit.creditUnitId)
  const singular = named?.name ?? unit.name ?? 'credit'
  const plural = named?.pluralName ?? unit.pluralName ?? 'credits'
  return (quantity: number) => (quantity === 1 ? singular : plural)
}

// Usage, or a capacity fee's overage, priced in a credit unit: its amounts are credits, billed in
// money at `rate` only for what the unit's wallet does not cover.
const usagePricing = (
  fee: SubscriptionFee,
  currency: string,
  units: CreditUnitNames | undefined
) => {
  const unit = fee.creditUnit?.rate ? fee.creditUnit : undefined
  const name = unit && unitNamer(unit, units)
  return {
    price: (value: number | string) =>
      name
        ? `${formatUsage(Number(value))} ${name(Number(value))}`
        : formatCurrencyNoRounding(Number(value), currency),
    rateNote:
      unit && name
        ? `1 ${name(1)} = ${formatCurrencyNoRounding(Number(unit.rate), currency)}, billed when not covered by ${name(2)}`
        : undefined,
  }
}

const withNote = (text: string | undefined, note: string | undefined) =>
  [text, note].filter(Boolean).join('\n') || undefined

export const formatSubscriptionFee = (
  fee: SubscriptionFee | undefined,
  currency: string,
  units?: CreditUnitNames
): {
  type: string
  details: string
  amount: string
  breakdown?: string // Optional detailed breakdown for complex pricing
  // True when `details` is boilerplate that conveys no pricing information
  // beyond `amount` (e.g. "Flat rate fee"). Callers rendering both side-by-side
  // should hide `details` in that case.
  redundantDetails?: boolean
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
        redundantDetails: true,
      }
    }

    case 'oneTime': {
      const oneTimeFee = fee.fee.value
      const multi = oneTimeFee.quantity > 1
      return {
        type: 'One-time',
        details: multi ? `${oneTimeFee.quantity}x @ ${oneTimeFee.rate}` : 'Single payment',
        amount: formatCurrencyNoRounding(Number(oneTimeFee.total || oneTimeFee.rate), currency),
        redundantDetails: !multi,
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

    case 'credit': {
      const credit = fee.fee.value
      const name = fee.creditUnit && unitNamer(fee.creditUnit, units)
      const creditOf = (value: string) =>
        name
          ? `${formatUsage(Number(value))} ${name(Number(value))}`
          : formatCurrencyNoRounding(Number(value), currency)
      const rollover = credit.rollover
      const term =
        rollover?.expiry &&
        `${rollover.expiry.value} ${rollover.expiry.unit === CreditPackExpiry_Unit.DAY ? 'day' : 'month'}${rollover.expiry.value === 1 ? '' : 's'}`
      // Credits already name themselves; money reads "$50.00 of credit".
      const ofCredit = fee.creditUnit ? '' : ' of credit'
      const granted =
        credit.quantity > 1
          ? `${credit.quantity}x ${creditOf(credit.creditAmount)}${ofCredit}`
          : `${creditOf(credit.creditAmount)}${ofCredit}`
      return {
        type: 'Credit',
        details: credit.once ? `${granted}, once at start` : granted,
        amount: formatCurrencyNoRounding(Number(credit.total || credit.rate), currency),
        breakdown: credit.once
          ? "Once at start: granted on the subscription's first invoice only"
          : rollover && term
            ? `Unused credit rolls over: ${rollover.maxAmount ? `up to ${creditOf(rollover.maxAmount)}` : 'all of it'}, for ${term}`
            : undefined,
      }
    }

    case 'capacity': {
      const capacityFee = fee.fee.value
      const hasOverage = parseFloat(capacityFee.overageRate) > 0
      const overage = usagePricing(fee, currency, units)
      return {
        type: 'Capacity',
        details: 'Capacity',
        amount: formatCurrencyNoRounding(Number(capacityFee.rate), currency),
        breakdown: withNote(
          `${formatUsage(Number(capacityFee.included))} included${hasOverage ? `, then ${overage.price(capacityFee.overageRate)} per unit` : ''}`,
          hasOverage ? overage.rateNote : undefined
        ),
      }
    }

    case 'slot': {
      const slotFee = fee.fee.value
      const limits = []

      if (slotFee.minSlots !== undefined) {
        limits.push(`min: ${formatUsage(slotFee.minSlots)}`)
      }

      if (slotFee.maxSlots !== undefined) {
        limits.push(`max: ${formatUsage(slotFee.maxSlots)}`)
      }

      const limitStr = limits.length > 0 ? ` (${limits.join(', ')})` : ''

      return {
        type: 'Slot',
        details: `${formatUsage(slotFee.initialSlots)} ${slotFee.unit}(s)${limitStr}`,
        amount: `${formatCurrencyNoRounding(Number(slotFee.unitRate), currency)} per ${slotFee.unit}`,
      }
    }

    case 'usage': {
      const usageFee = fee.fee.value
      const { price, rateNote } = usagePricing(fee, currency, units)

      switch (usageFee.model?.case) {
        case 'perUnit': {
          const perUnitPrice = usageFee.model.value
          return {
            type: 'Usage',
            details: 'Per unit',
            amount: `${price(Number(perUnitPrice))} per unit`,
            breakdown: rateNote,
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
            ? `${price(minPrice)} per unit`
            : `${price(minPrice)}–${price(maxPrice)} per unit`
          // Breakdown: show all tiers
          const breakdown = rows.map((row, idx) => {
            const nextRow = rows[idx + 1]
            const rangeStart = Number(row.firstUnit)
            const rangeEnd = nextRow ? Number(nextRow.firstUnit) - 1 : '∞'
            const formatted = price(Number(row.unitPrice))
            return `${rangeStart}-${rangeEnd}: ${formatted}`
          }).join('\n')
          return {
            type: 'Usage',
            details: 'Tiered',
            amount: summary,
            breakdown: withNote(breakdown, rateNote),
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
            ? `${price(minPrice)} per unit`
            : `${price(minPrice)}–${price(maxPrice)} per unit`
          // Breakdown: show all tiers
          const breakdown = rows.map((row, idx) => {
            const nextRow = rows[idx + 1]
            const rangeStart = Number(row.firstUnit)
            const rangeEnd = nextRow ? Number(nextRow.firstUnit) - 1 : '∞'
            const formatted = price(Number(row.unitPrice))
            return `${rangeStart}-${rangeEnd}: ${formatted}`
          }).join('\n')
          return {
            type: 'Usage',
            details: 'Volume',
            amount: summary,
            breakdown: withNote(breakdown, rateNote),
          }
        }

        case 'package': {
          const pkg = usageFee.model.value
          return {
            type: 'Usage',
            details: 'Package',
            amount: `${price(Number(pkg.packagePrice))} / ${pkg.blockSize} units`,
            breakdown: rateNote,
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
            ? `${price(minPrice)} per unit`
            : `${price(minPrice)}–${price(maxPrice)} per unit`
          // Breakdown: show all matrix price points
          const breakdown = rows.map(row => {
            const dims = [row.dimension1]
            if (row.dimension2) dims.push(row.dimension2)
            const dimStr = dims.map(d => d?.value).join(' · ')
            const formatted = price(Number(row.perUnitPrice))
            return `${dimStr}: ${formatted}`
          }).join('\n')
          return {
            type: 'Usage',
            details: 'Matrix',
            amount: summary,
            breakdown: withNote(breakdown, rateNote),
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
  currency: string,
  units?: CreditUnitNames
): string => {
  if (!fee || !fee.fee.case) {
    return 'N/A'
  }

  const formatted = formatSubscriptionFee(fee, currency, units)
  return `${formatted.type}: ${formatted.amount}`
}
