import Decimal from 'decimal.js'

export interface MetricUnit {
  singular?: string | null
  plural?: string | null
  /** raw units per priced unit, e.g. 1_000_000 */
  conversionFactor?: number | null
}

const DEFAULT_SINGULAR = 'unit'
const DEFAULT_PLURAL = 'units'

const FACTOR_ABBREVIATIONS: Record<number, string> = {
  1_000: '1K',
  1_000_000: '1M',
  1_000_000_000: '1B',
}

const groupedFormat = new Intl.NumberFormat('en-US')
const fullFormat = new Intl.NumberFormat('en-US', { maximumFractionDigits: 6 })
const compactLargeFormat = new Intl.NumberFormat('en-US', {
  notation: 'compact',
  minimumFractionDigits: 1,
  maximumFractionDigits: 1,
})
const compactSmallFormat = new Intl.NumberFormat('en-US', { maximumFractionDigits: 2 })
// Sub-1 values would round to "0" with fixed fraction digits; keep a few significant ones.
const compactFractionFormat = new Intl.NumberFormat('en-US', { maximumSignificantDigits: 3 })

const singularOf = (unit: MetricUnit | undefined) =>
  unit?.singular?.trim() || unit?.plural?.trim() || DEFAULT_SINGULAR

const pluralOf = (unit: MetricUnit | undefined) =>
  unit?.plural?.trim() || unit?.singular?.trim() || DEFAULT_PLURAL

const effectiveFactor = (unit: MetricUnit | undefined): number => {
  const factor = unit?.conversionFactor
  return factor != null && Number.isFinite(factor) && factor > 0 ? factor : 1
}

export const unitPlural = (unit: MetricUnit | undefined): string => pluralOf(unit)

/** A quantity in priced units back to raw metric units: 1.5 at 1M per unit is 1,500,000. */
export function rawQuantity(
  pricedQuantity: number | string | bigint,
  unit: MetricUnit | undefined
): number | null {
  try {
    return new Decimal(String(pricedQuantity).trim()).mul(effectiveFactor(unit)).toNumber()
  } catch {
    return null
  }
}

/** A quantity in priced units as grouped raw metric units, without the noun: "1,500,000". */
export function formatRawQuantity(
  pricedQuantity: number | string | bigint | undefined,
  unit: MetricUnit | undefined
): string {
  if (pricedQuantity === undefined || String(pricedQuantity).trim() === '') return '—'
  const raw = rawQuantity(pricedQuantity, unit)
  return raw === null ? String(pricedQuantity) : fullFormat.format(raw)
}

export function unitNoun(quantity: number, unit: MetricUnit | undefined): string {
  return Math.abs(quantity) === 1 ? singularOf(unit) : pluralOf(unit)
}

export function formatPerUnit(unit: MetricUnit | undefined): string {
  const factor = effectiveFactor(unit)
  if (factor === 1) {
    return `per ${singularOf(unit)}`
  }
  const amount = FACTOR_ABBREVIATIONS[factor] ?? groupedFormat.format(factor)
  return `per ${amount} ${pluralOf(unit)}`
}

export function formatPricePerUnit(formattedPrice: string, unit: MetricUnit | undefined): string {
  return `${formattedPrice} ${formatPerUnit(unit)}`
}

export function formatMetricQuantity(
  quantity: number | string,
  unit: MetricUnit | undefined,
  options?: { compact?: boolean }
): string {
  const value =
    typeof quantity === 'number' ? quantity : quantity.trim() === '' ? NaN : Number(quantity)
  if (!Number.isFinite(value)) {
    return `${String(quantity)} ${pluralOf(unit)}`
  }

  let formatted: string
  if (!options?.compact) {
    formatted = fullFormat.format(value)
  } else if (Math.abs(value) >= 1000) {
    formatted = compactLargeFormat.format(value)
  } else if (value !== 0 && Math.abs(value) < 1) {
    formatted = compactFractionFormat.format(value)
  } else {
    formatted = compactSmallFormat.format(value)
  }

  return `${formatted} ${unitNoun(value, unit)}`
}

export function metricUnitFromProto(
  metric:
    | {
        unitLabel?: { singular: string; plural: string }
        unitConversionFactor?: number
        aggregation?: { unitConversion?: { factor: number } }
      }
    | undefined
): MetricUnit | undefined {
  if (!metric) return undefined
  return {
    singular: metric.unitLabel?.singular || null,
    plural: metric.unitLabel?.plural || null,
    conversionFactor:
      metric.unitConversionFactor ?? metric.aggregation?.unitConversion?.factor ?? null,
  }
}
