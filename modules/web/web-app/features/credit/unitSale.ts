import { z } from 'zod'

import { parseMinorUnitsAtPrecision } from '@/features/credit/amounts'
import { currencyPrecision } from '@/lib/data/currency-precision'

export const MAX_UNIT_PRICE_DECIMALS = 12

const UNIT_PRICE_PATTERN = new RegExp(`^\\d+(\\.\\d{1,${MAX_UNIT_PRICE_DECIMALS}})?$`)

/** A price per credit: a positive decimal in major units of a currency. */
export const unitPriceSchema = z.string().superRefine((value, ctx) => {
  const trimmed = value.trim()
  if (!trimmed) {
    ctx.addIssue({ code: 'custom', message: 'Enter a price per credit' })
  } else if (!UNIT_PRICE_PATTERN.test(trimmed)) {
    ctx.addIssue({
      code: 'custom',
      message: `Enter a price with at most ${MAX_UNIT_PRICE_DECIMALS} decimals, e.g. 0.001`,
    })
  } else if (!/[1-9]/.test(trimmed)) {
    ctx.addIssue({ code: 'custom', message: 'The price must be above zero' })
  }
})

export const isValidUnitPrice = (value: string) => unitPriceSchema.safeParse(value).success

const decimalParts = (value: string): { digits: bigint; scale: number } => {
  const [whole, fraction = ''] = value.trim().split('.')
  return { digits: BigInt(`${whole}${fraction}`), scale: fraction.length }
}

export const sameUnitPrice = (a: string, b: string): boolean => {
  if (!isValidUnitPrice(a) || !isValidUnitPrice(b)) return a.trim() === b.trim()
  const left = decimalParts(a)
  const right = decimalParts(b)
  return left.digits * 10n ** BigInt(right.scale) === right.digits * 10n ** BigInt(left.scale)
}

/** Whether price `a` is at most price `b`; false when either is not a valid price. */
export const unitPriceAtMost = (a: string, b: string): boolean => {
  if (!isValidUnitPrice(a) || !isValidUnitPrice(b)) return false
  const left = decimalParts(a)
  const right = decimalParts(b)
  return left.digits * 10n ** BigInt(right.scale) <= right.digits * 10n ** BigInt(left.scale)
}

/**
 * Money (minor units of `currency`) for `creditsMinor` at `unitPrice` per credit, rounded once
 * half away from zero exactly as the server prices a sale. `null` for an invalid price.
 */
export const creditsToMoneyMinor = (
  creditsMinor: bigint,
  unitPrecision: number,
  unitPrice: string,
  currency: string
): bigint | null => {
  if (creditsMinor < 0n || !isValidUnitPrice(unitPrice)) return null
  const price = decimalParts(unitPrice)
  const numerator = creditsMinor * price.digits * 10n ** BigInt(currencyPrecision(currency))
  const denominator = 10n ** BigInt(unitPrecision + price.scale)
  return (2n * numerator + denominator) / (2n * denominator)
}

const quantityMessage = (precision: number) =>
  precision === 0
    ? 'Enter a whole number of credits'
    : `Enter a number of credits with at most ${precision} decimals`

/** A positive quantity of credits at the unit's precision. */
export const quantitySchema = (precision: number) =>
  z.string().superRefine((value, ctx) => {
    if (!value.trim()) {
      ctx.addIssue({ code: 'custom', message: 'Enter a number of credits' })
      return
    }
    const minor = parseMinorUnitsAtPrecision(value, precision)
    if (minor === null || minor <= 0n) {
      ctx.addIssue({ code: 'custom', message: quantityMessage(precision) })
    }
  })

type UnitLabel = { precision: number; name: string; pluralName: string }

export const formatCreditQuantity = (minor: bigint, precision: number): string =>
  new Intl.NumberFormat(undefined, {
    minimumFractionDigits: 0,
    maximumFractionDigits: precision,
  }).format(Number(minor) / 10 ** precision)

/** "50,000 Halyard credits", with the singular name for exactly one credit. */
export const formatCredits = (minor: bigint, unit: UnitLabel): string =>
  `${formatCreditQuantity(minor, unit.precision)} ${
    minor === 10n ** BigInt(unit.precision) ? unit.name : unit.pluralName
  }`
