import Decimal from 'decimal.js'
import { atom, useAtomValue } from 'jotai'
import {
  ActivityIcon,
  ArmchairIcon,
  ArrowDownIcon,
  Clock4Icon,
  CoinsIcon,
  ParkingMeterIcon,
  UngroupIcon,
} from 'lucide-react'
import { match } from 'ts-pattern'

import { priceWords } from '@/features/plans/canvas/billingWords'
import { usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { FIXED_MODEL_NAMES, fixedQuantityTotal } from '@/features/pricing/fixedQuantity'
import { unitSizeLabel } from '@/features/productCatalog/ratecards/rateTable'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import type { ComponentFeeType } from '@/features/pricing/conversions'
import type { CreditUnit } from '@/rpc/api/creditunits/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'
import type { LucideIcon } from 'lucide-react'

export const editedComponentsAtom = atom<string[]>([])

/** A cell to price, asked from outside the pricing section (the selling gaps banner). */
export interface PriceCellRequest {
  componentId: string
  currency: string
  /** None: the option shown. */
  optionId?: string
}

export const priceCellRequestAtom = atom<PriceCellRequest | undefined>(undefined)

export const useEditedComponents = () => {
  return useAtomValue(editedComponentsAtom)
}

export const formatPrice = (currency: string) => (price: string) => {
  const amountFloat = parseFloat(price)

  // A credit unit code (4+ characters) is no ISO currency: shown after the amount.
  try {
    return amountFloat.toLocaleString(undefined, {
      style: 'currency',
      currency,
      minimumFractionDigits: 2,
      maximumFractionDigits: 8,
    })
  } catch {
    return `${amountFloat.toLocaleString(undefined, { maximumFractionDigits: 8 })} ${currency}`
  }
}

export const useCurrency = () => {
  const { version } = usePlanWithVersion()
  return version?.currency ?? 'USD'
}

export const COMMON_CURRENCIES = ['USD', 'EUR', 'GBP', 'CAD', 'AUD', 'CHF', 'JPY', 'INR', 'BRL', 'MXN']

export const mapCadence = (
  cadence: 'ANNUAL' | 'SEMIANNUAL' | 'QUARTERLY' | 'MONTHLY' | 'COMMITTED'
): string => {
  return match(cadence)
    .with('ANNUAL', () => 'Annual')
    .with('SEMIANNUAL', () => 'Semiannual')
    .with('MONTHLY', () => 'Monthly')
    .with('QUARTERLY', () => 'Quarterly')
    .with('COMMITTED', () => 'Committed')
    .exhaustive()
}

export const feeTypeToHuman = (
  type: ComponentFeeType
) => {
  return match(type)
    .with('rate', () => 'Flat fee')
    .with('slot', () => 'Per seat')
    .with('capacity', () => 'Usage with included amount')
    .with('usage', () => 'Usage')
    .with('oneTime', () => 'One-time')
    .with('extraRecurring', () => 'Recurring by quantity')
    .with('credit', () => 'Credit pack')
    .exhaustive()
}

export function feeTypeIcon(type: ComponentFeeType): LucideIcon {
  switch (type) {
    case 'rate': return UngroupIcon
    case 'slot': return ArmchairIcon
    case 'capacity': return ParkingMeterIcon
    case 'usage': return ActivityIcon
    case 'oneTime': return ArrowDownIcon
    case 'extraRecurring': return Clock4Icon
    case 'credit': return CoinsIcon
  }
}

function cadenceShortLabel(cadence: BillingPeriod): string {
  switch (cadence) {
    case BillingPeriod.MONTHLY: return 'mo'
    case BillingPeriod.QUARTERLY: return 'qtr'
    case BillingPeriod.SEMIANNUAL: return '6mo'
    case BillingPeriod.ANNUAL: return 'yr'
  }
}

function priceRange(prices: number[], fmt: (p: string) => string): string {
  const min = Math.min(...prices)
  const max = Math.max(...prices)
  return min === max ? fmt(String(min)) : `${fmt(String(min))}–${fmt(String(max))}`
}

export function priceSummaryBadges(
  feeType: ComponentFeeType,
  latestPrice?: Price,
  currency?: string,
  creditUnits?: readonly Pick<CreditUnit, 'id' | 'name' | 'pluralName'>[]
): string[] {
  // Credits read in their unit's own name ("500 fast requests") once the units are known.
  const credits = (unitId: string, amount: string) => {
    const unit = creditUnits?.find(u => u.id === unitId)
    const value = parseFloat(amount)
    return `${value} ${value === 1 ? (unit?.name ?? 'credit') : (unit?.pluralName ?? 'credits')}`
  }
  const moneyFmt = currency ? formatPrice(currency) : (p: string) => p
  const fmt = moneyFmt

  if (!latestPrice?.pricing?.case) return [feeTypeToHuman(feeType)]

  const cadence = cadenceShortLabel(latestPrice.cadence)

  switch (latestPrice.pricing.case) {
    case 'ratePricing':
      return [`${fmt(latestPrice.pricing.value.rate)} / ${cadence}`]
    case 'slotPricing':
      // A slot model (seats included, tiers) carries the real price; unitRate is its first tier.
      return latestPrice.pricing.value.model
        ? [priceWords(latestPrice)]
        : ['Unit price', `${fmt(latestPrice.pricing.value.unitRate)} / seat`]
    case 'capacityPricing':
      return [`${fmt(latestPrice.pricing.value.rate)} / ${cadence}`, 'Capacity']
    case 'usagePricing': {
      // Priced in a credit unit: the amounts are credits, not money.
      const unitId = latestPrice.creditUnit?.creditUnitId
      const fmt = unitId ? (p: string) => credits(unitId, p) : moneyFmt
      const model = latestPrice.pricing.value.model
      if (model.case === 'perUnit') return [`${fmt(model.value)} / unit`]
      if (model.case === 'tiered' || model.case === 'volume') {
        const label = model.case === 'tiered' ? 'Tiered' : 'Volume'
        const prices = model.value.rows.map(r => parseFloat(r.unitPrice)).filter(n => !isNaN(n))
        if (prices.length === 0) return [label]
        const range = priceRange(prices, fmt)
        return [`${label}: ${range} / unit`]
      }
      if (model.case === 'package') {
        return [`${fmt(model.value.packagePrice)} / ${model.value.blockSize} units`]
      }
      if (model.case === 'dimensionRates') {
        const prices = model.value.rates.map(r => parseFloat(r.rate)).filter(n => !isNaN(n))
        if (prices.length === 0) return ['By dimension']
        const range = priceRange(prices, fmt)
        const per = unitSizeLabel(model.value.unitSize)
        return [`${range} ${per === 'per 1' ? '/ unit' : per}, by dimension`]
      }
      return ['Usage']
    }
    case 'extraRecurringPricing': {
      const { unitPrice, quantity, model } = latestPrice.pricing.value
      const total = fixedQuantityTotal(Math.max(quantity, 1), unitPrice, model).toString()
      const how = model?.model.case ? [FIXED_MODEL_NAMES[model.model.case] ?? model.model.case] : []
      return [`${fmt(total)} / ${cadence}`, ...how]
    }
    case 'oneTimePricing': {
      const { unitPrice, quantity, model } = latestPrice.pricing.value
      const total = fixedQuantityTotal(Math.max(quantity, 1), unitPrice, model).toString()
      const how = model?.model.case ? [FIXED_MODEL_NAMES[model.model.case] ?? model.model.case] : []
      return [`${fmt(total)}`, ...how]
    }
    case 'creditPricing': {
      const { unitPrice, quantity, creditAmount, once } = latestPrice.pricing.value
      const charge = new Decimal(unitPrice).mul(quantity).toString()
      const credit = new Decimal(creditAmount).mul(quantity).toString()
      // A pack granting a credit unit grants credits, not money.
      const grantedUnitId = latestPrice.pricing.value.creditUnitId
      const granted = grantedUnitId
        ? `+${credits(grantedUnitId, credit)}`
        : `+${fmt(credit)} credit`
      if (new Decimal(unitPrice).isZero()) {
        return [once ? 'Included once at start' : `Included / ${cadence}`, granted]
      }
      return [once ? `${fmt(charge)} once at start` : `${fmt(charge)} / ${cadence}`, granted]
    }
    default:
      return [feeTypeToHuman(feeType)]
  }
}
