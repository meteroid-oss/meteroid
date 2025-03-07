import { CURRENCIES } from '../data/currencies'

export const formatUsage = (quantity: number) => {
  let rounded
  if (Math.abs(quantity) < 1) {
    rounded = Number(quantity.toFixed(12))
  } else if (Math.abs(quantity) < 100) {
    rounded = Number(quantity.toFixed(4))
  } else {
    rounded = Number(quantity.toFixed(2))
  }

  return rounded.toString() // TODO: consider toLocaleString?
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
