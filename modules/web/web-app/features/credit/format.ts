import Decimal from 'decimal.js'

import { formatCurrency } from '@/lib/utils/numbers'

import type { CreditUnitLabel } from '@/rpc/api/wallets/v1/models_pb'

/** What a wallet holds: money of `currency`, or credits of `creditUnit` (then `currency` is its code). */
export type WalletDenomination = {
  currency: string
  creditUnit?: Pick<CreditUnitLabel, 'name' | 'pluralName' | 'precision'>
  /** Credits print as bare numbers: the unit is already named around them. */
  bareUnit?: boolean
}

const unitQuantity = (minor: bigint, precision: number) =>
  new Decimal(minor.toString()).div(new Decimal(10).pow(precision))

/** "$975.00", or "5,620 Halyard credits" for a credit-unit wallet. */
export const formatWalletAmount = (
  minor: bigint,
  denomination: WalletDenomination,
  { signed = false, bare = false }: { signed?: boolean; bare?: boolean } = {}
): string => {
  const sign = signed && minor > 0n ? '+' : minor < 0n ? '−' : ''
  const absolute = minor < 0n ? -minor : minor
  const unit = denomination.creditUnit
  if (!unit) return `${sign}${formatCurrency(absolute, denomination.currency)}`
  const quantity = unitQuantity(absolute, unit.precision)
  const number = new Intl.NumberFormat(undefined, {
    minimumFractionDigits: 0,
    maximumFractionDigits: unit.precision,
  }).format(quantity.toNumber())
  if (bare || denomination.bareUnit) return `${sign}${number}`
  return `${sign}${number} ${creditUnitName(minor, unit)}`
}

/** "credit" for exactly one, else "credits". */
export const creditUnitName = (
  minor: bigint,
  unit: Pick<CreditUnitLabel, 'name' | 'pluralName' | 'precision'>
) => (unitQuantity(minor < 0n ? -minor : minor, unit.precision).eq(1) ? unit.name : unit.pluralName)

const singular = (minor: bigint, unit?: Pick<CreditUnitLabel, 'precision'>) =>
  !unit || minor === 10n ** BigInt(unit.precision)

/** "$20.00 rolls over" and "1 credit rolls over", but "250 credits roll over". */
export const rollsOver = (minor: bigint, unit?: Pick<CreditUnitLabel, 'precision'>) =>
  singular(minor, unit) ? 'rolls over' : 'roll over'

/** "$20.00 expires", but "250 credits expire". */
export const expires = (minor: bigint, unit?: Pick<CreditUnitLabel, 'precision'>) =>
  singular(minor, unit) ? 'expires' : 'expire'

/** "USD credit", or the unit's name for a credit-unit wallet. */
export const walletTitle = (wallet: WalletDenomination) =>
  wallet.creditUnit ? wallet.creditUnit.pluralName : `${wallet.currency} credit`
