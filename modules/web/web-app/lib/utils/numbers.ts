import Decimal from 'decimal.js'

import { CURRENCIES } from '../data/currencies'

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
    useGrouping: false,
    maximumFractionDigits: maxDecimals,
  })
}

export const formatCurrency = (amount: bigint | number, currencyCode: string) => {
  const currency = CURRENCIES[currencyCode]

  const precision = currency.precision

  const parsedAmount = Number(amount) / Math.pow(10, precision)

  return new Intl.NumberFormat(undefined, {
    style: 'currency',
    currency: currencyCode,
    currencyDisplay: 'narrowSymbol',
    minimumFractionDigits: precision,
    maximumFractionDigits: precision,
  }).format(parsedAmount)
}


export const minorToMajorUnit = (amount: bigint | number, currencyCode: string): number => {
  const currency = CURRENCIES[currencyCode]
  const precision = currency.precision
  
  return Number(amount) / Math.pow(10, precision)
}

export const majorToMinorUnit = (amount: number, currencyCode: string): bigint => {
  const currency = CURRENCIES[currencyCode]
  const precision = currency.precision
  
  return BigInt(Math.round(amount * Math.pow(10, precision)))
}

export const formatCurrencyNoRounding = (amount: string | number, currencyCode: string) => {
  const currency = CURRENCIES[currencyCode]

  const precision = currency.precision

  const parsedAmount = typeof amount === 'string' ? parseFloat(amount) : amount

  return new Intl.NumberFormat('en-UK', {
    style: 'currency',
    currency: currencyCode,
    currencyDisplay: 'narrowSymbol',
    minimumFractionDigits: precision,
    maximumFractionDigits: 12,
  }).format(parsedAmount)
}
