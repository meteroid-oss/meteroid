import Decimal from 'decimal.js'

import { currencyPrecision } from '../data/currency-precision'

/**
 * Converts a decimal rate string to a percentage number.
 * Uses decimal.js for precision.
 * Example: "0.07" -> 7, "0.075" -> 7.5
 */
export const rateToPercent = (rate: string | number): number => {
  return new Decimal(rate).mul(100).toNumber()
}

/**
 * Converts a percentage number to a decimal rate string.
 * Uses decimal.js for precision.
 * Example: 7 -> "0.07", 7.5 -> "0.075"
 */
export const percentToRate = (percent: number): string => {
  return new Decimal(percent).div(100).toString()
}

export const formatUsage = (quantity: number) => {
  let maxDecimals: number
  if (Math.abs(quantity) < 1) {
    maxDecimals = 12
  } else if (Math.abs(quantity) < 100) {
    maxDecimals = 4
  } else {
    maxDecimals = 2
  }

  return quantity.toLocaleString('en-US', {
    useGrouping: true,
    maximumFractionDigits: maxDecimals,
  })
}

/** `trimWholeAmounts` drops the decimals of whole amounts (`€800`, but `€72.50`). */
export const formatCurrency = (
  amount: bigint | number,
  currencyCode: string,
  { trimWholeAmounts = false }: { trimWholeAmounts?: boolean } = {}
) => {
  const precision = currencyPrecision(currencyCode)
  const scale = Math.pow(10, precision)
  const digits = trimWholeAmounts && Number(amount) % scale === 0 ? 0 : precision

  const parsedAmount = Number(amount) / scale

  try {
    return new Intl.NumberFormat(undefined, {
      style: 'currency',
      currency: currencyCode,
      currencyDisplay: 'narrowSymbol',
      minimumFractionDigits: digits,
      maximumFractionDigits: digits,
    }).format(parsedAmount)
  } catch {
    // A credit unit code is no ISO currency: show the amount followed by the code.
    return `${new Intl.NumberFormat(undefined, {
      minimumFractionDigits: digits,
      maximumFractionDigits: digits,
    }).format(parsedAmount)} ${currencyCode}`
  }
}

export const minorToMajorUnit = (amount: bigint | number, currencyCode: string): number => {
  const precision = currencyPrecision(currencyCode)

  return Number(amount) / Math.pow(10, precision)
}

export const majorToMinorUnit = (amount: number, currencyCode: string): bigint => {
  const precision = currencyPrecision(currencyCode)

  return BigInt(Math.round(amount * Math.pow(10, precision)))
}

// Keeps sub-cent unit prices readable. The 12 digit cap must match MAX_UNIT_PRICE_SCALE in
// meteroid-invoicing/src/format.rs, so the invoice page and the PDF show the same price.
export const formatCurrencyNoRounding = (amount: string | number, currencyCode: string) => {
  const precision = currencyPrecision(currencyCode)

  const parsedAmount = typeof amount === 'string' ? parseFloat(amount) : amount

  return new Intl.NumberFormat('en-UK', {
    style: 'currency',
    currency: currencyCode,
    currencyDisplay: 'narrowSymbol',
    minimumFractionDigits: precision,
    maximumFractionDigits: 12,
  }).format(parsedAmount)
}
