import { format } from 'date-fns'

import { eligibilitySummary, isOpen } from '@/features/eligibility/eligibility'

import type { Rule } from '@/features/eligibility/eligibility'
import type { CouponDiscount_FixedDiscount } from '@/rpc/api/coupons/v1/models_pb'

export interface AmountRow {
  currency: string
  amount: string
}

/** The fixed discount: the main amount, and the same discount in other currencies. */
export const fixedDiscountOf = (amount: string, currency: string, others: AmountRow[] = []) => ({
  amount,
  currency,
  amounts: others
    .map(r => ({ currency: r.currency, amount: r.amount.trim() }))
    .filter(r => r.currency && r.currency !== currency),
})

/** A fixed discount's amounts in currencies other than its main one. */
export const otherAmountsOf = (
  fixed: Pick<CouponDiscount_FixedDiscount, 'currency' | 'amounts'>
): AmountRow[] =>
  fixed.amounts
    .filter(a => a.currency !== fixed.currency)
    .map(a => ({ currency: a.currency, amount: a.amount }))

// TODO time & timezone, to UTC (as the expiry always was)
export const couponDate = (date: Date | undefined) =>
  date ? format(date, "y-MM-dd'T'HH:mm:ss") : undefined

export const parseCouponDate = (value: string | undefined) => (value ? new Date(value) : undefined)

export interface CouponTerms {
  validFrom?: Date
  expiresAt?: Date
  redemptionLimit?: number
  reusable?: boolean
  otherAmounts?: AmountRow[]
  creditExpiresInDays?: number
  creditSubscriptionOnly?: boolean
  addOnCount?: number
  eligibility?: Rule
}

/** Whether a condition is set, so the disclosure opens on it. */
export const hasCouponTerms = (terms: CouponTerms) => couponTermsSummary(terms) !== ''

/** What the conditions set, in one line: "From 1 Dec · EUR 10 · Brazil only"; empty: none. */
export const couponTermsSummary = (
  terms: CouponTerms,
  country: (code: string) => string = code => code
): string => {
  const parts: string[] = []
  if (terms.validFrom) parts.push(`From ${format(terms.validFrom, 'd MMM')}`)
  if (terms.expiresAt) parts.push(`Until ${format(terms.expiresAt, 'd MMM')}`)
  // A cleared number input holds '', hence the truthiness checks.
  if (terms.redemptionLimit) {
    const limit = terms.redemptionLimit
    parts.push(`Max ${limit} ${limit === 1 ? 'redemption' : 'redemptions'}`)
  }
  if (terms.reusable) parts.push('Reusable')
  for (const row of terms.otherAmounts ?? []) {
    if (row.currency && row.amount.trim()) parts.push(`${row.currency} ${row.amount.trim()}`)
  }
  if (terms.creditExpiresInDays) {
    const days = terms.creditExpiresInDays
    parts.push(`Credit expires after ${days} ${days === 1 ? 'day' : 'days'}`)
  }
  if (terms.creditSubscriptionOnly) parts.push('One subscription only')
  if (terms.addOnCount) {
    parts.push(terms.addOnCount === 1 ? '1 add-on' : `${terms.addOnCount} add-ons`)
  }
  const rule = terms.eligibility
  if (rule && !isOpen(rule)) {
    parts.push(
      rule.countries.length && !rule.properties.length
        ? `${eligibilitySummary(rule, country).replace(/^Only customers in /, '')} only`
        : eligibilitySummary(rule, country)
    )
  }
  return parts.join(' · ')
}
