import { minorUnitsToInputAtPrecision } from '@/features/credit/amounts'
import { currencyPrecision } from '@/lib/data/currency-precision'
import { CreditExhaustedRule_Kind } from '@/rpc/api/plans/v1/models_pb'

import type { CreditUnit } from '@/rpc/api/creditunits/v1/models_pb'
import type { CreditExhaustedRule } from '@/rpc/api/plans/v1/models_pb'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'

export type CreditRuleKey = 'INVOICE' | 'PAUSE' | 'INVOICE_UP_TO'

export const creditRuleLabels: Record<CreditExhaustedRule_Kind, string> = {
  [CreditExhaustedRule_Kind.INVOICE]: 'Invoice the overage',
  [CreditExhaustedRule_Kind.PAUSE]: 'Deny access at zero',
  [CreditExhaustedRule_Kind.INVOICE_UP_TO]: 'Invoice overage up to a cap, then deny access',
}

/** A currency or credit unit the plan charges usage in, which a cap is expressed in. */
export type Denomination = { code: string; precision: number; label: string }

const prices = (components: PriceComponent[]): Price[] =>
  components.flatMap(component => component.prices)

const chargesUsage = (price: Price) =>
  price.pricing.case === 'usagePricing' || price.pricing.case === 'capacityPricing'

/** The rule only governs usage: a version that charges none has nothing to stop or invoice. */
export const versionChargesUsage = (components: PriceComponent[]): boolean =>
  prices(components).some(chargesUsage)

/**
 * Deny access at zero when the version grants credit (a pack, included or paid) or prices usage in
 * a credit unit, as the server does when it publishes a version whose rule was not chosen.
 */
export const defaultCreditRule = (components: PriceComponent[]): CreditRuleKey =>
  prices(components).some(price => price.creditUnit || price.pricing.case === 'creditPricing')
    ? 'PAUSE'
    : 'INVOICE'

export const creditRuleKey = (kind: CreditExhaustedRule_Kind): CreditRuleKey =>
  CreditExhaustedRule_Kind[kind] as CreditRuleKey

/** The denominations the version charges usage in, the plan currency when it charges none. */
export const usageDenominations = (
  components: PriceComponent[],
  currency: string,
  units: CreditUnit[] | undefined
): Denomination[] => {
  const found = new Map<string, Denomination>()
  for (const price of prices(components).filter(chargesUsage)) {
    const unit = price.creditUnit
      ? units?.find(candidate => candidate.id === price.creditUnit?.creditUnitId)
      : undefined
    if (price.creditUnit && !unit) continue
    const denomination = unit
      ? { code: unit.code.toUpperCase(), precision: unit.precision, label: unit.pluralName }
      : { code: currency.toUpperCase(), precision: currencyPrecision(currency), label: currency }
    found.set(denomination.code, denomination)
  }
  if (found.size === 0) {
    found.set(currency.toUpperCase(), {
      code: currency.toUpperCase(),
      precision: currencyPrecision(currency),
      label: currency,
    })
  }
  return [...found.values()]
}

/** "Invoice overage up to 500 EUR, then deny access"; the default is said as such. */
export const creditRuleSummary = (
  rule: CreditExhaustedRule | undefined,
  components: PriceComponent[],
  denominations: Denomination[]
): string => {
  if (!rule)
    return `${creditRuleLabels[CreditExhaustedRule_Kind[defaultCreditRule(components)]]} (default)`
  if (rule.kind !== CreditExhaustedRule_Kind.INVOICE_UP_TO) return creditRuleLabels[rule.kind]
  const caps = rule.caps.map(cap => {
    const denomination = denominations.find(candidate => candidate.code === cap.denomination)
    const amount = minorUnitsToInputAtPrecision(cap.amount, denomination?.precision ?? 0)
    return `${amount} ${denomination?.label ?? cap.denomination}`
  })
  return caps.length > 0
    ? `Invoice overage up to ${caps.join(' / ')}, then deny access`
    : creditRuleLabels[rule.kind]
}
