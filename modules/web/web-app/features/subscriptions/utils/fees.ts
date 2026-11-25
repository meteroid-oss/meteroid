
import { countModelWords, fixedQuantityModelWords } from "@/features/plans/canvas/billingWords"
import { fixedQuantityTotal } from "@/features/pricing/fixedQuantity"
import { unitSizeLabel } from "@/features/productCatalog/ratecards/rateTable"
import { CreditPackExpiry_Unit } from "@/rpc/api/prices/v1/models_pb"
import {
  SubscriptionFee,
  SubscriptionFeeBillingPeriod,
  SubscriptionFee_FeeCreditUnit,
} from "@/rpc/api/subscriptions/v1/models_pb"
import { formatCurrencyNoRounding, formatUsage } from "@/utils/numbers"

import type { CreditUnit } from "@/rpc/api/creditunits/v1/models_pb"
import type { UsagePricing } from "@/rpc/api/prices/v1/models_pb"
import type { SubscriptionFee_SlotSubscriptionFee } from "@/rpc/api/subscriptions/v1/models_pb"
import type Decimal from "decimal.js"

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

const capitalized = (text: string) => text.charAt(0).toUpperCase() + text.slice(1)

/** A model that prices the count itself (tiers, volume, package), not one rate per unit. */
const countModel = (model: UsagePricing | undefined) => (model?.model.case ? model : undefined)

type SlotFee = Pick<
  SubscriptionFee_SlotSubscriptionFee,
  'unit' | 'unitRate' | 'model' | 'minSlots' | 'maxSlots'
>

/** The seats a count of `held` bills: within the fee's minimum and maximum, as the engine bills. */
export const billedSlots = (slot: Pick<SlotFee, 'minSlots' | 'maxSlots'>, held: number) =>
  Math.min(Math.max(held, slot.minSlots ?? 0), slot.maxSlots ?? Number.POSITIVE_INFINITY)

/** What `held` seats cost a period: their rate each, or their count priced through the model. */
export const slotFeeTotal = (slot: SlotFee, held: number): Decimal =>
  fixedQuantityTotal(billedSlots(slot, held), slot.unitRate, countModel(slot.model))

/** Whether no count of seats costs anything: a zero rate, or a model with nothing but zeros. */
export const slotFeeIsFree = (slot: Pick<SlotFee, 'unitRate' | 'model'>): boolean => {
  const zero = (amount?: string) => !amount || Number(amount) === 0
  const model = countModel(slot.model)?.model
  switch (model?.case) {
    case undefined:
      return zero(slot.unitRate)
    case 'perUnit':
      return zero(model.value)
    case 'tiered':
    case 'volume':
      return model.value.rows.every(row => zero(row.unitPrice) && zero(row.flatFee))
    case 'package':
      return zero(model.value.packagePrice)
    default:
      return false
  }
}

/** A seat price in words: "$10.00 per seat", or its model's "First 3 free, then $10 per seat". */
export const slotFeeWords = (slot: SlotFee, currency: string): string => {
  const model = countModel(slot.model)
  return model
    ? capitalized(countModelWords(model, currency, slot.unit))
    : `${formatCurrencyNoRounding(Number(slot.unitRate || 0), currency)} per ${slot.unit}`
}

/**
 * How a fixed quantity is priced, when more than one unit at one price: "3 × $9.99", or its
 * model's "15 units: first 10 at $10, then $8 each". Undefined for a single unit.
 */
export const fixedQuantityDetails = (
  value: { rate: string; quantity: number; model?: UsagePricing },
  currency: string
): string | undefined => {
  const model = countModel(value.model)
  if (model) return fixedQuantityModelWords(value.quantity, model, currency)
  return value.quantity > 1
    ? `${formatUsage(value.quantity)} × ${formatCurrencyNoRounding(Number(value.rate || 0), currency)}`
    : undefined
}

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
      const how = fixedQuantityDetails(oneTimeFee, currency)
      return {
        type: 'One-time',
        details: how ?? 'Single payment',
        amount: formatCurrencyNoRounding(Number(oneTimeFee.total || oneTimeFee.rate), currency),
        redundantDetails: !how,
      }
    }

    case 'recurring': {
      const recurringFee = fee.fee.value
      const billingType = recurringFee.billingType === 0 ? 'in arrears' : 'in advance'
      const how = fixedQuantityDetails(recurringFee, currency)
      return {
        type: 'Recurring',
        details: how ? `${how} (${billingType})` : `Recurring ${billingType}`,
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
        amount: slotFeeWords(slotFee, currency),
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
          // Breakdown: show all tiers, each its own units, the first counted from 1
          // (billing-core tiered_charges); volume tiers below are read by totals.
          const breakdown = rows.map((row, idx) => {
            const nextRow = rows[idx + 1]
            const rangeStart = Number(row.firstUnit) + 1
            const rangeEnd = nextRow ? Number(nextRow.firstUnit) : '∞'
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

        case 'dimensionRates': {
          const rates = usageFee.model.value
          if (rates.rates.length === 0) {
            return { type: 'Usage', details: 'By dimension', amount: '-' }
          }
          const per = unitSizeLabel(rates.unitSize)
          const unit = per === 'per 1' ? 'per unit' : per
          // Summary: show price range
          const prices = rates.rates.map(r => Number(r.rate))
          const minPrice = Math.min(...prices)
          const maxPrice = Math.max(...prices)
          const summary = minPrice === maxPrice
            ? `${price(minPrice)} ${unit}`
            : `${price(minPrice)}–${price(maxPrice)} ${unit}`
          // Breakdown: every key and its rate, `*` read as any value
          const breakdown = rates.rates.map(rate => {
            const dimStr = rate.key.map(p => (p === '*' ? 'any' : p)).join(' · ')
            return `${dimStr}: ${price(Number(rate.rate))}`
          }).join('\n')
          return {
            type: 'Usage',
            details: 'By dimension',
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
