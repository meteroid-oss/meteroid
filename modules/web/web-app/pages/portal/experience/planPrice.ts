import { formatPerUnit } from '@/features/billing/metrics/units'
import { rollsOver } from '@/features/credit/format'

import {
  cadenceAdjective,
  cadenceNoun,
  cadenceSuffix,
  compactNumber,
  decimalStr,
  everyCadence,
  money,
  rateStr,
} from './format'
import { creditCount, creditsLabel, unitNameFor } from './walletFormat'

import type {
  CreditTerm,
  IncludedCredit,
  PlanPrice,
  UsageRates,
} from '@/rpc/portal/subscription/v1/subscription_pb'

export const PAY_AS_YOU_GO = 'Pay as you go'

/** No recurring fee and no minimum: the plan bills only its usage. */
export const isPayAsYouGo = (price: PlanPrice) =>
  price.usageBased && price.amount === 0n && !!price.usageRates && price.minimum === undefined

/** No recurring fee, only a minimum per period: headlined by that minimum. */
export const isMinimumOnly = (price: PlanPrice) =>
  price.amount === 0n && price.minimum !== undefined && price.minimum > 0n

/** Nothing recurring to pay: no fee, no minimum, and usage is not billed. */
export const isFreePlan = (price: PlanPrice) =>
  price.amount === 0n && !isPayAsYouGo(price) && !isMinimumOnly(price)

/** Net of discounts: "$500.00", the minimum on a minimum-only plan, or "$0" for a free plan. */
export const planPriceAmount = (price: PlanPrice) =>
  isMinimumOnly(price)
    ? money(price.minimum!, price.currency)
    : money(price.amount, price.currency, { compact: price.amount === 0n })

/** The price before discounts, when a discount applies. */
export const planListAmount = (price: PlanPrice) =>
  price.discounts.length > 0 && price.listAmount !== undefined && price.listAmount !== price.amount
    ? money(price.listAmount, price.currency, { compact: price.listAmount === 0n })
    : undefined

/** "$500 / month" (whole amount, as in a plan comparison), "$5,000 / month minimum", or "Pay as you go". */
export const planPriceShort = (price: PlanPrice) => {
  if (isPayAsYouGo(price)) return PAY_AS_YOU_GO
  const minimumOnly = isMinimumOnly(price)
  const amount = money(minimumOnly ? price.minimum! : price.amount, price.currency, {
    compact: true,
  })
  return `${amount} ${cadenceSuffix(price.cadence)}${minimumOnly ? ' minimum' : ''}`
}

const singular = (name: string) => (/[^s]s$/.test(name) ? name.slice(0, -1) : name)

/** "per 1M tokens" from the metric's unit, else from its name: "per 1M model tokens". */
const usagePer = (rates: UsageRates) => {
  if (rates.unitLabel) {
    return formatPerUnit({ ...rates.unitLabel, conversionFactor: Number(rates.unitSize) })
  }
  const metric = rates.metricName?.toLowerCase()
  if (rates.unitSize <= 1n) return `per ${metric ? singular(metric) : 'unit'}`
  return `per ${compactNumber(rates.unitSize.toString())} ${metric ?? 'units'}`
}

const rateAmount = (rates: UsageRates, rate: string) =>
  rates.creditUnit
    ? `${decimalStr(rate)} ${Number(rate) === 1 ? rates.creditUnit.name : creditsLabel(rates.creditUnit)}`
    : rateStr(rate, rates.currency)

/** "$2.00 – $60.00 per 1M model tokens", a single rate when there is one. */
export const usageRatesLine = (rates: UsageRates) => {
  const same = Number(rates.minRate) === Number(rates.maxRate)
  const range = same
    ? rateAmount(rates, rates.minRate)
    : rates.creditUnit
      ? `${decimalStr(rates.minRate)} – ${decimalStr(rates.maxRate)} ${creditsLabel(rates.creditUnit)}`
      : `${rateStr(rates.minRate, rates.currency)} – ${rateStr(rates.maxRate, rates.currency)}`
  return `${range} ${usagePer(rates)}`
}

/** On top of a recurring fee: "+ usage from $2.00 per 1M model tokens". */
const extraUsageLine = (rates: UsageRates) =>
  Number(rates.minRate) === Number(rates.maxRate)
    ? `+ usage at ${rateAmount(rates, rates.minRate)} ${usagePer(rates)}`
    : `+ usage from ${rateAmount(rates, rates.minRate)} ${usagePer(rates)}`

/** "20% off · $10.00 off". */
export const discountLine = (price: PlanPrice) =>
  price.discounts
    .map(discount =>
      discount.percentage !== undefined
        ? `${decimalStr(discount.percentage)}% off`
        : discount.amount !== undefined
          ? `${money(discount.amount, price.currency)} off`
          : ''
    )
    .filter(Boolean)
    .join(' · ')

const creditAmount = (credit: IncludedCredit) =>
  credit.creditUnit
    ? `${creditCount(credit.amount, credit.creditUnit)} ${unitNameFor(credit.amount, credit.creditUnit)}`
    : `${money(credit.amount, credit.currency)} of credit`

/** "Includes $550.00 of credit every month" / "Includes 5,000 credits every month". */
export const includedCreditLine = (credit: IncludedCredit) =>
  `Includes ${creditAmount(credit)} ${everyCadence(credit.cadence)}`

const plural = (n: number, noun: string) => `${n} ${noun}${n === 1 ? '' : 's'}`

const termLength = (term?: CreditTerm) =>
  term?.months !== undefined
    ? plural(term.months, 'month')
    : term?.days !== undefined
      ? plural(term.days, 'day')
      : undefined

/** How long unused included credit lasts, e.g. "Unused credit carries over for 12 months". */
export const creditTermLine = (credit: IncludedCredit): string | undefined => {
  const length = termLength(credit.expiry)
  if (length) return `Unused credit carries over for ${length}`
  if (!credit.expiry?.periodEnd) return undefined
  if (!credit.rollover) {
    return `Unused credit expires at the end of each ${cadenceNoun(credit.cadence) || 'period'}`
  }
  const rolloverLength = termLength(credit.rollover.expiry)
  const lasting = rolloverLength ? ` for ${rolloverLength}` : ''
  const max = credit.rollover.maxAmount
  if (max === undefined) return `Unused credit rolls over${lasting}`
  return credit.creditUnit
    ? `Up to ${creditCount(max, credit.creditUnit)} unused ${unitNameFor(max, credit.creditUnit)} ${rollsOver(max, credit.creditUnit)}${lasting}`
    : `Up to ${money(max, credit.currency)} of unused credit rolls over${lasting}`
}

/** The lines under a plan price: discounts, usage rates, included credit, minimum, other fees. */
export const planPriceLines = (
  price: PlanPrice,
  description?: string,
  opts: { creditTerms?: boolean } = {}
): string[] => {
  const lines: string[] = []
  const discounts = discountLine(price)
  if (discounts) lines.push(discounts)
  // Usage counts towards a minimum, so its rates are listed rather than added on top.
  const usageOnly = isPayAsYouGo(price) || isMinimumOnly(price)
  if (usageOnly && price.usageRates) lines.push(usageRatesLine(price.usageRates))
  price.includedCredits.forEach(credit => {
    lines.push(includedCreditLine(credit))
    const term = opts.creditTerms ? creditTermLine(credit) : undefined
    if (term) lines.push(term)
  })
  if (price.minimum !== undefined && !isMinimumOnly(price)) {
    lines.push(`${money(price.minimum, price.currency)} ${cadenceAdjective(price.cadence)} minimum`)
  }
  price.otherCadences.forEach(other =>
    lines.push(`+ ${money(other.amount, price.currency)} ${cadenceSuffix(other.cadence)}`)
  )
  if (price.usageRates && !usageOnly) lines.push(extraUsageLine(price.usageRates))
  if (price.includedCredits.length === 0 && price.minimum === undefined && description) {
    lines.push(description)
  }
  return lines
}

/** "per month" under a recurring price, "minimum per month" under a minimum, "one-time" otherwise. */
export const perCadence = (price: PlanPrice) => {
  const noun = cadenceNoun(price.cadence)
  if (!noun) return 'one-time'
  return isMinimumOnly(price) ? `minimum per ${noun}` : `per ${noun}`
}

/** What follows the headline amount: "/ month", "/ month minimum" or "one-time". */
export const headlineSuffix = (price: PlanPrice) => {
  const noun = cadenceNoun(price.cadence)
  if (!noun) return 'one-time'
  return isMinimumOnly(price) ? `/ ${noun} minimum` : `/ ${noun}`
}
