import { cardRatesLabel } from '@/features/productCatalog/ratecards/markup'
import { dimensionName, listNames, unitSizeLabel } from '@/features/productCatalog/ratecards/rateTable'

import { normalizeShape } from './compile'

import type { Cadence, ChargeShape, ChargeValues } from './compile'

const PERIOD: Record<Cadence, string> = {
  MONTHLY: 'month',
  QUARTERLY: 'quarter',
  SEMIANNUAL: 'half-year',
  ANNUAL: 'year',
}

const ADVERB: Record<Cadence, string> = {
  MONTHLY: 'monthly',
  QUARTERLY: 'quarterly',
  SEMIANNUAL: 'every 6 months',
  ANNUAL: 'yearly',
}

export interface SummaryContext {
  currency: string
  metricName?: string
  /** The credit unit a usage price is expressed in, when not money. */
  creditUnitName?: string
  /** What a usage price is per ("AI Assist Action"), when the metric names its unit. */
  unitSingular?: string
}

export function formatMoney(amount: unknown, currency: string): string {
  const value = Number(amount ?? 0)
  if (!isFinite(value)) return String(amount ?? '')
  try {
    return value.toLocaleString(undefined, {
      style: 'currency',
      currency,
      minimumFractionDigits: 2,
      maximumFractionDigits: 8,
    })
  } catch {
    return `${value.toLocaleString(undefined, { maximumFractionDigits: 8 })} ${currency}`
  }
}

const count = (value: unknown) => Number(value ?? 0).toLocaleString()

/** "Seats" → "seat": a unit name read in the singular. Words like "access" stay as they are. */
export function singular(name: string): string {
  const lower = name.trim().toLowerCase()
  if (!lower) return 'unit'
  if (/[^aeiou]ies$/.test(lower)) return `${lower.slice(0, -3)}y`
  if (/(ss|sh|ch|x|z)es$/.test(lower)) return lower.slice(0, -2)
  if (lower.length > 2 && /[^sui]s$/.test(lower)) return lower.slice(0, -1)
  return lower
}

function firstTierPrice(rows: unknown): string | undefined {
  const first = Array.isArray(rows) ? (rows[0] as { unitPrice?: string } | undefined) : undefined
  return first?.unitPrice || undefined
}

/** One line read back to the operator: "€12.00 per seat per month, billed in advance". */
export function summarizeCharge(
  shape: ChargeShape,
  values: ChargeValues,
  context: SummaryContext
): string {
  const s = normalizeShape(shape)
  const term = (values.term as Cadence | undefined) ?? 'MONTHLY'
  const per = `per ${PERIOD[term]}`
  const m = (amount: unknown) => formatMoney(amount, context.currency)
  const usage = (amount: unknown) =>
    context.creditUnitName ? `${count(amount)} ${context.creditUnitName}` : m(amount)
  const timing =
    s.recurrence === 'one_time' ? 'once, on the first invoice' : `${per}, billed in ${s.billing}`
  const tiers = s.model === 'tiered' ? 'graduated tiers' : 'volume tiers'

  switch (s.source) {
    case 'fixed': {
      const quantity = count(values.quantity ?? 1)
      switch (s.model) {
        case 'flat':
          return `${m(values.rate)} ${timing}`
        case 'per_unit':
          return `${quantity} × ${m(values.unitPrice)} ${timing}`
        case 'tiered':
        case 'volume': {
          const from = firstTierPrice(values.rows)
          return `${quantity} units on ${tiers}${from ? ` from ${m(from)} per unit` : ''} ${timing}`
        }
        case 'package':
          return `${quantity} units in packs of ${count(values.blockSize)} at ${m(values.packagePrice)} ${timing}`
        case 'credit_pack': {
          const packs = Number(values.quantity ?? 1)
          const what = `${count(packs)} ${packs === 1 ? 'pack' : 'packs'} of ${count(values.creditAmount)} credit`
          const when = s.recurrence === 'one_time' ? 'once at start' : per
          return Number(values.unitPrice ?? 0) === 0
            ? `${what} included ${when}`
            : `${what} at ${m(values.unitPrice)} ${when}`
        }
        default:
          return ''
      }
    }
    case 'seats': {
      const unitName = (values.slotUnitName as string | undefined) || 'Seats'
      const unit = singular(unitName)
      const limits = [
        values.minSlots ? `at least ${count(values.minSlots)}` : '',
        values.maxSlots ? `at most ${count(values.maxSlots)}` : '',
      ]
        .filter(Boolean)
        .join(', ')
      const tail = `${per}, billed in advance${limits ? `; ${limits} ${unitName.toLowerCase()}` : ''}`
      switch (s.model) {
        case 'per_unit':
          return `${m(values.unitPrice)} per ${unit} ${tail}`
        case 'tiered':
        case 'volume': {
          const from = firstTierPrice(values.rows)
          return `${unitName} on ${tiers}${from ? ` from ${m(from)} per ${unit}` : ''} ${tail}`
        }
        case 'package':
          return `${m(values.packagePrice)} per ${count(values.blockSize)} ${unitName.toLowerCase()} ${tail}`
        default:
          return ''
      }
    }
    case 'metered': {
      const of = context.metricName ? ` of ${context.metricName}` : ''
      const unit = context.unitSingular ?? 'unit'
      const arrears = `billed ${ADVERB[term]} in arrears`
      switch (s.model) {
        case 'per_unit':
          return `${usage(values.unitPrice)} per ${context.unitSingular ?? `unit${of}`}, ${arrears}`
        case 'tiered':
        case 'volume': {
          const from = firstTierPrice(values.rows)
          return `Usage${of} on ${tiers}${from ? ` from ${usage(from)} per ${unit}` : ''}, ${arrears}`
        }
        case 'package':
          return `${usage(values.packagePrice)} per ${count(values.blockSize)} units${of}, ${arrears}`
        case 'percentage':
          return `${values.percent || 0}% of the amount${of}, ${arrears}`
        case 'dynamic':
          return `The amount${of}, priced by its events, ${arrears}`
        case 'by_dimension': {
          if (values.rateSource === 'card') {
            const rates = cardRatesLabel(String(values.markup ?? ''))
            return `Usage${of} at its rate card's rates${rates === 'card rates' ? '' : ` (${rates})`}, ${arrears}`
          }
          // "Usage of Tokens by Model and Token type, 3 rates per 1M units, billed ..."
          const names = ((values.dimensions as string[] | undefined) ?? []).filter(d => d.trim())
          const rates = ((values.keyRates as unknown[] | undefined) ?? []).length
          const by = names.length ? ` by ${listNames(names.map(dimensionName))}` : ' by dimension'
          const per = unitSizeLabel(String(values.unitSize ?? '1'), {
            singular: unit,
            plural: `${unit}s`,
          })
          return `Usage${of}${by}, ${count(rates)} ${rates === 1 ? 'rate' : 'rates'} ${per}, ${arrears}`
        }
        case 'included': {
          const levels = Array.isArray(values.thresholds)
            ? (values.thresholds as { included?: number; rate?: string; overageRate?: string }[])
            : []
          const first = levels[0]
          if (!first) return ''
          const more = levels.length > 1 ? ` (${levels.length} levels)` : ''
          return `${m(first.rate)} ${per} with ${count(first.included)} included, then ${usage(first.overageRate)} per ${context.unitSingular ?? `unit${of}`}${more}; fee in advance, overage in arrears`
        }
        default:
          return ''
      }
    }
  }
}
