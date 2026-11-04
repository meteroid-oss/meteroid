import Decimal from 'decimal.js'
import { z } from 'zod'

import { currencyPrecision } from '@/lib/data/currency-precision'

/**
 * Exact major-unit input to minor units at an explicit precision; `null` when it is not a valid
 * amount. Use this (not `parseMinorUnits`) wherever the precision cannot come from the global
 * currency/credit-unit catalog — e.g. the portal, which never loads the tenant's unit catalog.
 */
export const parseMinorUnitsAtPrecision = (value: string, precision: number): bigint | null => {
  const trimmed = value.trim()
  if (!/^\d+(\.\d+)?$/.test(trimmed)) return null
  const minor = new Decimal(trimmed).mul(new Decimal(10).pow(precision))
  return minor.isInteger() ? BigInt(minor.toFixed(0)) : null
}

export const minorUnitsToInputAtPrecision = (
  minor: bigint | undefined,
  precision: number
): string =>
  minor === undefined
    ? ''
    : new Decimal(minor.toString()).div(new Decimal(10).pow(precision)).toFixed()

/** Exact major-unit input to minor units; `null` when it is not a valid amount in the currency. */
export const parseMinorUnits = (value: string, currency: string): bigint | null =>
  parseMinorUnitsAtPrecision(value, currencyPrecision(currency))

export const minorUnitsToInput = (minor: bigint | undefined, currency: string): string =>
  minorUnitsToInputAtPrecision(minor, currencyPrecision(currency))

const amountMessage = (currency: string) => {
  const precision = currencyPrecision(currency)
  return precision === 0
    ? `Enter a whole amount of ${currency}`
    : `Enter an amount with at most ${precision} decimals`
}

export const amountSchema = (currency: string) =>
  z.string().superRefine((value, ctx) => {
    if (!value.trim()) {
      ctx.addIssue({ code: 'custom', message: 'Enter an amount' })
      return
    }
    const minor = parseMinorUnits(value, currency)
    if (minor === null || minor <= 0n) {
      ctx.addIssue({ code: 'custom', message: amountMessage(currency) })
    }
  })

/** Empty is accepted; zero is a valid limit. */
export const optionalAmountSchema = (currency: string) =>
  z
    .string()
    .refine(
      value => !value.trim() || parseMinorUnits(value, currency) !== null,
      amountMessage(currency)
    )

export const optionalMinorUnits = (value: string, currency: string): bigint | undefined =>
  value.trim() ? (parseMinorUnits(value, currency) ?? undefined) : undefined

/** Comma-separated amounts to minor units, ascending and distinct; throws on anything else. */
export const parseMinorUnitsList = (value: string, currency: string, max: number): bigint[] => {
  const trimmed = value.trim()
  if (!trimmed) return []
  const minors = trimmed.split(',').map(part => parseMinorUnits(part, currency))
  if (minors.some(minor => minor === null) || minors.length > max) {
    throw new Error(`Enter up to ${max} increasing amounts of ${currency}, e.g. 5, 20, 50`)
  }
  const values = minors as bigint[]
  values.forEach((minor, index) => {
    if (index > 0 && minor <= values[index - 1]) {
      throw new Error(`Enter up to ${max} increasing amounts of ${currency}, e.g. 5, 20, 50`)
    }
  })
  return values
}

export const minorUnitsListToInput = (minors: bigint[], currency: string): string =>
  minors.map(minor => minorUnitsToInput(minor, currency)).join(', ')

/**
 * A few round, currency-sized quick-pick amounts for a top-up form. Zero-decimal currencies (e.g.
 * JPY) use larger round units so the picks stay sensible at that currency's scale.
 */
export const topUpPresets = (currency: string): bigint[] => {
  const unit = new Decimal(10).pow(currencyPrecision(currency))
  const majorAmounts =
    currencyPrecision(currency) === 0 ? [1000, 2500, 5000, 10000] : [25, 50, 100, 250]
  return majorAmounts.map(major => BigInt(unit.mul(major).toFixed(0)))
}

/** Comma-separated, ascending, distinct amounts; empty clears the list. */
export const amountListSchema = (currency: string, max: number) =>
  z.string().superRefine((value, ctx) => {
    try {
      parseMinorUnitsList(value, currency, max)
    } catch (error) {
      ctx.addIssue({ code: 'custom', message: (error as Error).message })
    }
  })
