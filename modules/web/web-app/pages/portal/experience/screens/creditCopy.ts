import Decimal from 'decimal.js'

import { isValidUnitPrice } from '@/features/credit/unitSale'
import { currencyPrecision } from '@/lib/data/currency-precision'

import type { CreditUnitLabel, Wallet } from '@/rpc/portal/wallets/v1/models_pb'

/** Who the customer asks when the portal offers no action: the merchant by name, else "us". */
export const contactName = (merchantName?: string) => merchantName?.trim() || 'us'

/** What the customer's credit pays, for the Credit page: credits pay usage, money pays invoices. */
export const creditPaysLine = (wallets: Pick<Wallet, 'creditUnit'>[]) => {
  const units = wallets.some(wallet => wallet.creditUnit)
  const money = wallets.some(wallet => !wallet.creditUnit)
  if (units && money) return 'Applied to your usage and invoices automatically.'
  return units ? 'Applied to your usage automatically.' : 'Applied to your invoices automatically.'
}

/** When a bought top-up lands and what it pays, as the top-up modal and hosted page say it. */
export const topUpSettlesCopy = (unit?: Pick<CreditUnitLabel, 'pluralName'>) =>
  unit
    ? `the ${unit.pluralName} are added once the payment settles. They pay your usage.`
    : 'the credit is added once the payment settles. It pays your next invoices.'

const NICE_STEPS = [1, 2, 2.5, 5]

// The nearest 1-2-2.5-5 number of whole credits, so a preset reads 25,000 rather than 31,000.
const roundCount = (credits: Decimal): bigint => {
  if (credits.lte(1)) return 1n
  const exponent = Math.floor(Math.log10(credits.toNumber()))
  const candidates = [...NICE_STEPS.map(step => step * 10 ** exponent), 10 ** (exponent + 1)]
    .filter(Number.isInteger)
    .map(value => new Decimal(value))
  const distance = (value: Decimal) => Math.abs(Math.log(value.div(credits).toNumber()))
  const best = candidates.reduce((a, b) => (distance(b) < distance(a) ? b : a))
  return BigInt(best.toFixed(0))
}

/**
 * Round quick-pick quantities worth about the usual money presets at `unitPrice` (e.g. 25,000 /
 * 50,000 / 100,000 / 250,000 credits at $0.0008), in minor units of the unit.
 */
export const roundCreditPresets = (
  unitPrice: string,
  unitPrecision: number,
  currency: string
): bigint[] => {
  if (!isValidUnitPrice(unitPrice)) return []
  const majorAmounts =
    currencyPrecision(currency) === 0 ? [1000, 2500, 5000, 10000] : [25, 50, 100, 250]
  const scale = 10n ** BigInt(unitPrecision)
  const presets = majorAmounts.map(major => roundCount(new Decimal(major).div(unitPrice)) * scale)
  return presets.filter((preset, index) => index === 0 || preset > presets[index - 1])
}
