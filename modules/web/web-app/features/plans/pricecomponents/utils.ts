import { atom, useAtomValue } from 'jotai'
import {
  ActivityIcon,
  ArmchairIcon,
  ArrowDownIcon,
  Clock4Icon,
  ParkingMeterIcon,
  UngroupIcon,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import { match } from 'ts-pattern'

import type { ComponentFeeType } from '@/features/pricing/conversions'
import { usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import type { Price } from '@/rpc/api/prices/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

export const editedComponentsAtom = atom<string[]>([])

export const useEditedComponents = () => {
  return useAtomValue(editedComponentsAtom)
}

export const formatPrice = (currency: string) => (price: string) => {
  const amountFloat = parseFloat(price)

  return amountFloat.toLocaleString(undefined, {
    style: 'currency',
    currency,
    minimumFractionDigits: 2,
    maximumFractionDigits: 8,
  })
}

export const useCurrency = () => {
  const { version } = usePlanWithVersion()
  return version?.currency ?? 'USD'
}

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
    .with('rate', () => 'Subscription Rate')
    .with('slot', () => 'Slot-based')
    .with('capacity', () => 'Capacity commitment')
    .with('usage', () => 'Usage-based')
    .with('oneTime', () => 'One-time charge')
    .with('extraRecurring', () => 'Recurring charge')
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

export function priceSummaryBadges(
  feeType: ComponentFeeType,
  latestPrice?: Price,
  currency?: string,
): string[] {
  const fmt = currency ? formatPrice(currency) : (p: string) => p

  if (!latestPrice?.pricing?.case) return [feeTypeToHuman(feeType)]

  const cadence = cadenceShortLabel(latestPrice.cadence)

  switch (latestPrice.pricing.case) {
    case 'ratePricing':
      return [`${fmt(latestPrice.pricing.value.rate)} / ${cadence}`]
    case 'slotPricing':
      return ['UNIT PRICE', `${fmt(latestPrice.pricing.value.unitRate)} / seat`]
    case 'capacityPricing':
      return [`${fmt(latestPrice.pricing.value.rate)} / ${cadence}`, 'CAPACITY']
    case 'usagePricing': {
      const model = latestPrice.pricing.value.model
      if (model.case === 'perUnit') return ['PER UNIT', `${fmt(model.value)} / unit`]
      if (model.case === 'tiered') return ['TIERED']
      if (model.case === 'volume') return ['VOLUME']
      if (model.case === 'package') return ['PACKAGE']
      if (model.case === 'matrix') return ['MATRIX']
      return ['USAGE']
    }
    case 'extraRecurringPricing':
      return [`${fmt(latestPrice.pricing.value.unitPrice)} / ${cadence}`]
    case 'oneTimePricing':
      return [`${fmt(latestPrice.pricing.value.unitPrice)}`]
    default:
      return [feeTypeToHuman(feeType)]
  }
}
