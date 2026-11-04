import Decimal from 'decimal.js'

import type { CreditUnit } from '@/rpc/api/creditunits/v1/models_pb'

const positive = (value: string | undefined): value is string => {
  if (!value?.trim()) return false
  try {
    return new Decimal(value).gt(0)
  } catch {
    return false
  }
}

/** What one credit of `unit` is worth in `currency`: its live selling price there, if any. */
export const creditValue = (
  unit: Pick<CreditUnit, 'prices'> | undefined,
  currency: string
): string | undefined => {
  const code = currency.trim().toUpperCase()
  const price = unit?.prices.find(
    price => !price.archivedAt && price.currency.toUpperCase() === code
  )?.unitPrice
  return positive(price) ? price : undefined
}

/**
 * The money one credit stands for under a price: its own overage rate when it sets one, else the
 * unit's value in the currency (as the server bills it where usage past zero is invoiced).
 */
export const effectiveCreditRate = (
  priceRate: string | undefined,
  unit: Pick<CreditUnit, 'prices'> | undefined,
  currency: string
): string | undefined => (positive(priceRate) ? priceRate : creditValue(unit, currency))

/** "≈ 0.009 per credit (10% off 0.01)": a pack's price per credit against the unit's value. */
export const packCreditPrice = (
  packPrice: string,
  creditsPerPack: string,
  value: string | undefined
): { perCredit: string; discountPct?: number } | undefined => {
  try {
    const price = new Decimal(packPrice || 0)
    const credits = new Decimal(creditsPerPack || 0)
    if (price.lte(0) || credits.lte(0)) return undefined
    const perCredit = price.div(credits)
    if (!positive(value)) return { perCredit: perCredit.toSignificantDigits(4).toString() }
    const discount = new Decimal(1).minus(perCredit.div(value)).mul(100)
    return {
      perCredit: perCredit.toSignificantDigits(4).toString(),
      discountPct: discount.toDecimalPlaces(0).toNumber(),
    }
  } catch {
    return undefined
  }
}
