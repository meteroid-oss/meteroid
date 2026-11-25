import Decimal from 'decimal.js'

import {
  formatMetricQuantity,
  formatPerUnit,
  rawQuantity,
  unitNoun,
} from '@/features/billing/metrics/units'

import { normalizeShape } from './compile'
import { isAllowanceTiers, sortedLevels, tierBreakdown, tierRange } from './kinds'
import { formatMoney } from './summary'

import type { ChargeShape, ChargeValues } from './compile'
import type { LevelRow, TierRowLike } from './kinds'
import type { MetricUnit } from '@/features/billing/metrics/units'

// What a charge puts on an invoice at a sample quantity, for the form's "On the invoice" column.
// Pure, as billing-core prices it (kinds.ts customerPays, tierBreakdown).

export interface PreviewContext {
  /** The invoice line's name: the charge's. */
  name: string
  currency: string
  /** "month". */
  per: string
  /** "monthly". */
  adverb: string
  /** The seat, in the singular: "seat". */
  seat: string
  unit?: MetricUnit
  metricName?: string
  /** Usage priced in this credit unit, not in money. */
  creditUnitName?: string
}

export interface PreviewSample {
  /** Seats, or usage in priced units. */
  quantity: number
  /** The level the customer picked, by its rank. */
  level: number
  /** An invoice after the first: a one-time fee is no longer on it. */
  later: boolean
}

export interface PreviewLine {
  label: string
  detail?: string
  amount: Decimal
  /** A part of the line above it: a tier, the usage above a level. */
  sub?: boolean
}

export interface InvoicePreview {
  lines: PreviewLine[]
  total: Decimal
  totalLabel: string
}

const dec = (value: unknown) => {
  try {
    return new Decimal(String(value ?? '').trim() || '0')
  } catch {
    return new Decimal(0)
  }
}
const grouped = (value: Decimal.Value) => new Decimal(value).toNumber().toLocaleString('en-US')
const plural = (n: number, one: string, many: string) => (n === 1 ? one : many)

/** Whether the charge bills once: a one-time fee, a credit pack granted once. */
export const billsOnce = (shape: ChargeShape) => normalizeShape(shape).recurrence === 'one_time'

/** The invoice lines a charge produces for `sample`; none when it has no one quantity to price. */
export function invoicePreview(
  shape: ChargeShape,
  values: ChargeValues,
  sample: PreviewSample,
  ctx: PreviewContext
): InvoicePreview | undefined {
  const s = normalizeShape(shape)
  const m = (amount: unknown) => formatMoney(amount, ctx.currency)
  // Usage is priced in money, or in the credit unit its price names.
  const u = (amount: unknown) =>
    ctx.creditUnitName ? `${dec(amount).toString()} ${ctx.creditUnitName}` : m(amount)
  // Usage in priced units: "2,400 calls", "2.4M tokens" when a unit is a million of them.
  const usage = (n: Decimal.Value) => {
    const value = new Decimal(n).toNumber()
    if ((ctx.unit?.conversionFactor ?? 1) === 1)
      return `${grouped(value)} ${unitNoun(value, ctx.unit)}`
    return formatMetricQuantity(rawQuantity(value, ctx.unit) ?? value, ctx.unit, { compact: true })
  }
  const perUnit = formatPerUnit(ctx.unit)
  const seats = (n: number) => `${grouped(n)} ${plural(n, ctx.seat, `${ctx.seat}s`)}`
  const q = new Decimal(Math.max(0, sample.quantity || 0))
  const once = s.recurrence === 'one_time'
  const totalLabel = once ? (sample.later ? 'Next invoices' : 'First invoice') : `Each ${ctx.per}`
  const result = (lines: PreviewLine[]): InvoicePreview => ({
    lines,
    total: lines
      .filter(line => !line.sub)
      .reduce((sum, line) => sum.add(line.amount), new Decimal(0)),
    totalLabel,
  })
  const main = (detail: string | undefined, amount: Decimal): PreviewLine => ({
    label: ctx.name,
    detail,
    amount,
  })

  /** The tiers billed for `count`, each a part of the line. */
  const tierLines = (
    count: Decimal,
    what: (n: Decimal.Value) => string,
    rate: (a: unknown) => string
  ) => {
    const rows = (values.rows as TierRowLike[] | undefined) ?? []
    const mode = s.model === 'volume' ? 'volume' : 'graduated'
    const { lines, total } = tierBreakdown(rows, count, mode)
    const parts = lines.map((line): PreviewLine => {
      const fee = line.fee.isZero() ? '' : ` + ${rate(line.fee)}`
      return {
        label:
          mode === 'graduated'
            ? `${grouped(line.from)} – ${grouped(new Decimal(line.from).add(line.units).sub(1))}`
            : `All ${what(line.units)}`,
        detail:
          mode === 'graduated'
            ? `${what(line.units)} at ${rate(line.unitPrice)}${fee}`
            : `at ${rate(line.unitPrice)}${fee}, ${tierRange(line.from, line.to, 'volume')}`,
        amount: line.amount,
        sub: true,
      }
    })
    return { parts, total }
  }

  switch (s.source) {
    case 'fixed': {
      // A one-time fee or pack is on the first invoice only.
      if (once && sample.later) {
        return result([main('Charged on the first invoice only', new Decimal(0))])
      }
      const count = Number(values.quantity ?? 1) || 1
      switch (s.model) {
        case 'flat':
          return result([main(once ? 'One-time fee' : undefined, dec(values.rate))])
        case 'per_unit':
          return result([
            main(`${grouped(count)} × ${m(values.unitPrice)}`, dec(values.unitPrice).mul(count)),
          ])
        case 'tiered':
        case 'volume': {
          const { parts, total } = tierLines(new Decimal(count), n => `${grouped(n)} units`, m)
          return result([main(`${grouped(count)} units`, total), ...parts])
        }
        case 'package': {
          const size = dec(values.blockSize)
          if (size.isZero()) return undefined
          const packs = new Decimal(count).div(size).ceil()
          return result([
            main(
              `${grouped(packs)} ${plural(packs.toNumber(), 'package', 'packages')} of ${grouped(size)} × ${m(values.packagePrice)}`,
              packs.mul(dec(values.packagePrice))
            ),
          ])
        }
        case 'credit_pack':
          return result([
            main(
              `${grouped(count)} × ${grouped(dec(values.creditAmount))} credits`,
              dec(values.unitPrice).mul(count)
            ),
          ])
        default:
          return undefined
      }
    }
    case 'seats': {
      const n = q.floor().toNumber()
      switch (s.model) {
        case 'per_unit':
          return result([
            main(`${seats(n)} × ${m(values.unitPrice)}`, dec(values.unitPrice).mul(n)),
          ])
        case 'tiered':
        case 'volume': {
          const { parts, total } = tierLines(
            new Decimal(n),
            x => seats(new Decimal(x).toNumber()),
            m
          )
          return result([main(seats(n), total), ...parts])
        }
        case 'package': {
          const size = dec(values.blockSize)
          if (size.isZero()) return undefined
          const packs = new Decimal(n).div(size).ceil()
          return result([
            main(
              `${seats(n)}: ${grouped(packs)} ${plural(packs.toNumber(), 'package', 'packages')} × ${m(values.packagePrice)}`,
              packs.mul(dec(values.packagePrice))
            ),
          ])
        }
        default:
          return undefined
      }
    }
    case 'metered': {
      switch (s.model) {
        case 'per_unit':
          return result([
            main(`${usage(q)} at ${u(values.unitPrice)} ${perUnit}`, q.mul(dec(values.unitPrice))),
          ])
        case 'tiered':
        case 'volume': {
          if (s.model === 'tiered' && isAllowanceTiers(values.rows)) {
            const [, then] = values.rows as TierRowLike[]
            const included = dec(then.firstUnit)
            const over = Decimal.max(q.sub(included), 0)
            const overage = over.mul(dec(then.unitPrice))
            const parts: PreviewLine[] = [
              {
                label: `First ${usage(included)}`,
                detail: 'Included',
                amount: new Decimal(0),
                sub: true,
              },
            ]
            if (over.greaterThan(0)) {
              parts.push({
                label: `${usage(over)} above it`,
                detail: `at ${u(then.unitPrice)} ${perUnit}`,
                amount: overage,
                sub: true,
              })
            }
            return result([main(usage(q), overage), ...parts])
          }
          const { parts, total } = tierLines(q, usage, u)
          return result([main(usage(q), total), ...parts])
        }
        case 'package': {
          const size = dec(values.blockSize)
          if (size.isZero()) return undefined
          const packs = q.div(size).ceil()
          return result([
            main(
              `${usage(q)}: ${grouped(packs)} ${plural(packs.toNumber(), 'package', 'packages')} of ${usage(size)} × ${u(values.packagePrice)}`,
              packs.mul(dec(values.packagePrice))
            ),
          ])
        }
        case 'included': {
          const levels = sortedLevels((values.thresholds as LevelRow[] | undefined) ?? [])
          const level = levels[Math.min(Math.max(sample.level, 0), levels.length - 1)]
          if (!level) return undefined
          const over = Decimal.max(q.sub(level.included ?? 0), 0)
          const lines: PreviewLine[] = [
            main(`Up to ${usage(level.included ?? 0)}`, dec(level.rate)),
          ]
          if (over.greaterThan(0)) {
            lines.push({
              label: `${usage(over)} above it`,
              detail: `at ${u(level.overageRate)} ${perUnit}`,
              amount: over.mul(dec(level.overageRate)),
            })
          }
          return result(lines)
        }
        default:
          return undefined
      }
    }
  }
}

/** When the charge is billed, in one line: "Billed monthly, in advance · counted by Tokens". */
export function timingCaption(
  shape: ChargeShape,
  ctx: Pick<PreviewContext, 'per' | 'adverb' | 'metricName'>
): string {
  const s = normalizeShape(shape)
  const when = (() => {
    switch (s.source) {
      case 'fixed':
        if (s.recurrence === 'one_time') {
          return s.model === 'credit_pack'
            ? 'Granted once, on the first invoice'
            : 'Billed once, on the first invoice'
        }
        if (s.model === 'credit_pack') return `Granted ${ctx.adverb}, billed in advance`
        return s.billing === 'advance'
          ? `Billed ${ctx.adverb}, in advance`
          : `Billed ${ctx.adverb}, after each ${ctx.per}`
      case 'seats':
        return `Billed ${ctx.adverb}, in advance`
      case 'metered':
        return s.model === 'included'
          ? `Fee billed in advance · usage above it after each ${ctx.per}`
          : `Billed after each ${ctx.per}`
    }
  })()
  return s.source === 'metered' && ctx.metricName ? `${when} · counted by ${ctx.metricName}` : when
}

/** Two significant digits: 2,400 and 0.25 rather than 2,437.5 and 0.2513. */
const nice = (n: number) => (n > 0 ? Number(n.toPrecision(2)) : 0)

/**
 * The quantity the example starts at, and the top of its slider: around what the price sets, else
 * a thousand units (ten when a unit is itself many, such as a million tokens).
 */
export function sampleRange(
  shape: ChargeShape,
  values: ChargeValues,
  unit?: MetricUnit
): { initial: number; max: number } {
  const s = normalizeShape(shape)
  if (s.source === 'seats') return { initial: 10, max: 100 }
  if (s.model === 'included') {
    const levels = sortedLevels((values.thresholds as LevelRow[] | undefined) ?? [])
    const first = Number(levels[0]?.included ?? 0)
    const last = Number(levels[levels.length - 1]?.included ?? 0)
    return { initial: nice(first * 1.2) || 1000, max: nice(Math.max(last * 1.5, 1000)) }
  }
  if (s.model === 'tiered' || s.model === 'volume') {
    const rows = (values.rows as TierRowLike[] | undefined) ?? []
    const last = Number(rows[rows.length - 1]?.firstUnit ?? 0)
    if (isAllowanceTiers(rows)) {
      return { initial: nice(last * 2.4) || 1000, max: nice(Math.max(last * 4, 1000)) }
    }
    return { initial: nice(last * 1.5) || 1000, max: nice(Math.max(last * 3, 2000)) }
  }
  return (unit?.conversionFactor ?? 1) > 1
    ? { initial: 10, max: 100 }
    : { initial: 1000, max: 10000 }
}
