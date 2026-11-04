import { currencyPrecision } from '@/lib/data/currency-precision'

export const amountFormat = ({ total, currency }: { total: bigint, currency: string }) => {
  if (typeof total !== 'bigint') return ''
  const precision = currencyPrecision(currency)
  return new Intl.NumberFormat(navigator.language, {
    style: 'currency',
    currency: currency,
    minimumFractionDigits: precision,
    maximumFractionDigits: precision,
  }).format(Number(total) / Math.pow(10, precision))
}
