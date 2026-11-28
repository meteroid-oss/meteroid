import Decimal from 'decimal.js'

import { bestFit, normalizeShape, shapeFits, valuesForShape } from './compile'
import { formatMoney, singular, summarizeCharge } from './summary'

import type { Cadence, ChargeLock, ChargeShape, ChargeValues, Recurrence } from './compile'

// The kinds an operator picks a charge by (ChargeEdit / ChargeAdd artboards). Each is a preset
// of the compile.ts shape; the form then reaches the other rating models. A fixed fee is charged
// every period or once; usage may include some each period (a free first tier).

export type ChargeKind = 'fixed' | 'seat' | 'usage' | 'levels' | 'included' | 'credit'

export const KIND_LABELS: Record<ChargeKind, string> = {
  fixed: 'A fixed fee',
  seat: 'Seats',
  usage: 'Usage',
  levels: 'Usage levels',
  included: 'Included, no charge',
  credit: 'Credit pack',
}

/** The kinds offered by "What it charges for"; a credit pack keeps its own editor. */
export const MENU_KINDS: ChargeKind[] = ['fixed', 'seat', 'usage', 'levels', 'included']

export interface TierRowLike {
  firstUnit: bigint | number | string
  unitPrice: string
  flatFee?: string
  flatCap?: string
}

const isZero = (value: unknown) => {
  try {
    return new Decimal(String(value ?? '0').trim() || '0').isZero()
  } catch {
    return false
  }
}
const isBlank = (value: unknown) => value === undefined || value === null || value === ''

/** Two graduated tiers, the first free: usage with an allowance ("1M free, then $0.0004"). */
export function isAllowanceTiers(rows: unknown): rows is [TierRowLike, TierRowLike] {
  if (!Array.isArray(rows) || rows.length !== 2) return false
  const [first, second] = rows as TierRowLike[]
  return (
    !isBlank(first.unitPrice) &&
    isZero(first.unitPrice) &&
    [first.flatFee, first.flatCap, second.flatFee, second.flatCap].every(isBlank)
  )
}

export const allowanceRows = (included: unknown, rate: unknown): TierRowLike[] => [
  { firstUnit: BigInt(0), unitPrice: '0' },
  { firstUnit: toBigInt(included), unitPrice: String(rate ?? '') },
]

const toBigInt = (value: unknown): bigint => {
  try {
    return BigInt(String(value ?? '0').replace(/[,\s_]/g, '') || '0')
  } catch {
    return BigInt(0)
  }
}

/** The kind a charge reads as, from its shape: one-time fees and allowances are of their kind. */
export function kindOf(shape: ChargeShape): ChargeKind {
  const s = normalizeShape(shape)
  switch (s.source) {
    case 'fixed':
      return s.model === 'credit_pack' ? 'credit' : 'fixed'
    case 'seats':
      return 'seat'
    case 'metered':
      return s.model === 'included' ? 'levels' : 'usage'
  }
}

const KIND_SHAPES: Record<Exclude<ChargeKind, 'included'>, ChargeShape> = {
  fixed: { source: 'fixed', model: 'flat', recurrence: 'recurring', billing: 'advance' },
  seat: { source: 'seats', model: 'per_unit', recurrence: 'recurring', billing: 'advance' },
  usage: { source: 'metered', model: 'per_unit', recurrence: 'recurring', billing: 'arrears' },
  // A fee per level of usage the customer picks, overage above it (a capacity fee).
  levels: { source: 'metered', model: 'included', recurrence: 'recurring', billing: 'advance' },
  credit: { source: 'fixed', model: 'credit_pack', recurrence: 'recurring', billing: 'advance' },
}

/**
 * The shape a kind compiles to for a product (`lock`), or undefined when the product cannot be
 * sold so (another kind of charge needs another product). A capacity product is sold in levels;
 * a one-time product as a fixed fee charged once.
 */
export function shapeForKind(kind: ChargeKind, lock?: ChargeLock): ChargeShape | undefined {
  if (kind === 'included') return undefined
  const base = KIND_SHAPES[kind]
  // Its model is what it is: another one fitting the product would be another kind.
  if (kind === 'levels') return shapeFits(base, lock) ? base : undefined
  const recurrences: Recurrence[] = kind === 'fixed' ? ['recurring', 'one_time'] : [base.recurrence]
  for (const recurrence of recurrences) {
    const fitted = bestFit({ ...base, recurrence }, { source: base.source, recurrence }, lock)
    if (!fitted || !shapeFits(fitted, lock)) continue
    // A fixed fee fitted to a credit product would be a credit pack, usage to a capacity one levels.
    if (kind === 'fixed' && fitted.model === 'credit_pack') continue
    if (kind === 'usage' && fitted.model === 'included') continue
    return fitted
  }
  return undefined
}

export const kindOffered = (kind: ChargeKind, lock?: ChargeLock) =>
  kind === 'included' || shapeForKind(kind, lock) !== undefined

/**
 * The shape and values after picking `kind`, or undefined when the product cannot be sold so.
 * Levels picked as usage keep the first level's allowance: that much included, then its overage.
 */
export function pickKind(
  kind: ChargeKind,
  lock: ChargeLock | undefined,
  current: ChargeValues,
  previous?: ChargeShape
): { shape: ChargeShape; values: ChargeValues } | undefined {
  const shape = shapeForKind(kind, lock)
  if (!shape) return undefined
  const level =
    previous?.model === 'included' ? (current.thresholds as LevelRow[] | undefined)?.[0] : undefined
  if (kind === 'usage' && level && Number(level.included ?? 0) > 0) {
    const tiered: ChargeShape = { ...shape, model: 'tiered' }
    if (shapeFits(tiered, lock)) {
      const values = valuesForShape(tiered, current, previous)
      values.rows = allowanceRows(level.included, level.overageRate ?? '0.00')
      return { shape: tiered, values }
    }
  }
  return { shape, values: valuesForKind(kind, shape, current, previous) }
}

/** The values after picking `kind` (shape `next`): kept where they fit, the rest filled in. */
export function valuesForKind(
  kind: ChargeKind,
  next: ChargeShape,
  current: ChargeValues,
  previous?: ChargeShape
): ChargeValues {
  const values = valuesForShape(next, current, previous)
  const level = (current.thresholds as LevelRow[] | undefined)?.[0]
  // A level's overage is what usage costs per unit.
  if (kind === 'usage' && next.model === 'per_unit' && previous?.model === 'included' && level) {
    values.unitPrice = level.overageRate ?? values.unitPrice
  }
  if (kind === 'levels' && previous?.model !== 'included' && isAllowanceTiers(current.rows)) {
    // An allowance becomes the first level, its fee to set.
    const [, then] = current.rows as TierRowLike[]
    values.thresholds = [
      {
        included: Number(toBigInt(then.firstUnit)),
        rate: '0.00',
        overageRate: then.unitPrice || '0',
      },
    ]
  }
  if (next.model === 'credit_pack') values.once = next.recurrence === 'one_time'
  return values
}

// --- Usage with some included each period ---

/** The usage included each period: the free first tier of an allowance, else none. */
export function includedUsage(shape: ChargeShape, values: ChargeValues): bigint {
  const s = normalizeShape(shape)
  if (s.source !== 'metered' || s.model !== 'tiered' || !isAllowanceTiers(values.rows)) {
    return BigInt(0)
  }
  return toBigInt((values.rows as TierRowLike[])[1].firstUnit)
}

/**
 * Usage priced per unit with `included` units free each period: stored as two graduated tiers,
 * the first free, or plain per-unit usage when none is included. The price per unit is kept.
 */
export function withIncludedUsage(
  shape: ChargeShape,
  values: ChargeValues,
  included: unknown
): { shape: ChargeShape; values: ChargeValues } {
  const s = normalizeShape(shape)
  const allowance = s.model === 'tiered' && isAllowanceTiers(values.rows)
  const rate = allowance
    ? (values.rows as TierRowLike[])[1].unitPrice
    : String(values.unitPrice ?? '0.00')
  const amount = toBigInt(included)
  if (amount <= BigInt(0)) {
    return { shape: { ...s, model: 'per_unit' }, values: { ...values, unitPrice: rate } }
  }
  return { shape: { ...s, model: 'tiered' }, values: { ...values, rows: allowanceRows(amount, rate) } }
}

// --- Levels: a fee for each amount of usage the customer may pick, overage above it ---

/** One level of a capacity charge, as its form keeps it. */
export interface LevelRow {
  included?: number
  rate?: string
  overageRate?: string
}

const sameAmount = (a: unknown, b: unknown) => dec(a).equals(dec(b))

/** The levels by the usage they include, smallest first. */
export const sortedLevels = (levels: LevelRow[]): LevelRow[] =>
  [...levels].sort((a, b) => Number(a.included ?? 0) - Number(b.included ?? 0))

/** Whether the levels bill usage above them at different rates: their sentence then says each. */
export const overageVaries = (levels: unknown): boolean => {
  if (!Array.isArray(levels) || levels.length < 2) return false
  const [first, ...others] = levels as LevelRow[]
  return others.some(level => !sameAmount(level.overageRate, first.overageRate))
}

/** A level after the largest: twice its usage, at its price and overage, to be set. */
export function addLevel(levels: LevelRow[]): LevelRow[] {
  const last = sortedLevels(levels)[levels.length - 1]
  const included = Number(last?.included ?? 0)
  return [
    ...levels,
    {
      included: included > 0 ? included * 2 : 1000,
      rate: last?.rate ?? '0.00',
      overageRate: last?.overageRate ?? '0.00000000',
    },
  ]
}

const CURRENCY_NAMES = (() => {
  try {
    return new Intl.DisplayNames(['en'], { type: 'currency' })
  } catch {
    return undefined
  }
})()

/**
 * Whether the metric counts money in `currency`: its unit is named as the currency ("EUR", "€",
 * "euros") and not scaled. Its usage is then an amount, and a rate per unit a share of it.
 */
export function unitIsCurrency(
  unit:
    | { singular?: string | null; plural?: string | null; conversionFactor?: number | null }
    | undefined,
  currency: string
): boolean {
  if (!unit || (unit.conversionFactor ?? 1) !== 1) return false
  const code = currency.trim().toUpperCase()
  const names = new Set<string>([code.toLowerCase()])
  try {
    const symbol = (0)
      .toLocaleString('en-US', {
        style: 'currency',
        currency: code,
        currencyDisplay: 'narrowSymbol',
      })
      .replace(/[\d.,\s]/g, '')
    if (symbol) names.add(symbol.toLowerCase())
  } catch {
    return false
  }
  const name = CURRENCY_NAMES?.of(code)?.toLowerCase()
  if (name && name !== code.toLowerCase()) {
    names.add(name)
    names.add(`${name}s`)
  }
  const labels = [unit.singular, unit.plural]
    .map(label => label?.trim().toLowerCase())
    .filter((label): label is string => !!label)
  return labels.length > 0 && labels.every(label => names.has(label))
}

/** A rate per unit of money as a share of it: 0.005 is "0.5%". */
export const percentOf = (rate: unknown) =>
  `${dec(rate).mul(100).toDecimalPlaces(6).toString()}%`

export interface SentenceContext {
  currency: string
  term: Cadence
  metricName?: string
  unitSingular?: string
  unitPlural?: string
  creditUnitName?: string
  /** The metric counts money in the currency (unitIsCurrency): overage reads as a percentage. */
  moneyUnit?: boolean
}

const PER: Record<Cadence, string> = {
  MONTHLY: 'month',
  QUARTERLY: 'quarter',
  SEMIANNUAL: 'half-year',
  ANNUAL: 'year',
}

const grouped = (value: unknown) => {
  const n = Number(String(value ?? '0'))
  return Number.isFinite(n) ? n.toLocaleString('en-US') : String(value)
}

const compactCount = new Intl.NumberFormat('en-US', {
  notation: 'compact',
  maximumFractionDigits: 1,
})

/** "a, b or c". */
const orList = (words: string[]) =>
  words.length < 2
    ? (words[0] ?? '')
    : `${words.slice(0, -1).join(', ')} or ${words[words.length - 1]}`

/**
 * Levels in words: "€479 a month for up to €125K, then 0.5% above it", several joined with "or"
 * as the customer picks one; each its own overage when they differ.
 */
function levelsSentence(levels: LevelRow[], ctx: SentenceContext): string {
  const per = PER[ctx.term]
  const m = (amount: unknown) => formatMoney(amount, ctx.currency)
  const amount = (included: unknown) => {
    const n = Number(included ?? 0)
    if (!ctx.moneyUnit) return `${compactCount.format(n)} ${ctx.unitPlural ?? 'units'}`
    try {
      return n.toLocaleString('en-US', {
        style: 'currency',
        currency: ctx.currency,
        notation: 'compact',
        minimumFractionDigits: 0,
        maximumFractionDigits: 1,
      })
    } catch {
      return `${compactCount.format(n)} ${ctx.currency}`
    }
  }
  const unit = ctx.unitSingular ?? 'unit'
  const overage = (rate: unknown) => {
    if (ctx.creditUnitName) return `${grouped(rate)} ${ctx.creditUnitName} per ${unit}`
    if (ctx.moneyUnit) return percentOf(rate)
    return `${m(rate)} per ${unit}`
  }
  const sorted = sortedLevels(levels)
  const own = overageVaries(sorted)
  const parts = sorted.map((level, i) => {
    const fee = i === 0 ? `${m(level.rate)} a ${per}` : m(level.rate)
    const then = own ? ` (then ${overage(level.overageRate)})` : ''
    return `${fee} for up to ${amount(level.included)}${then}`
  })
  const picks = sorted.length > 1 ? ', as the customer picks' : ''
  const then = own ? '' : `, then ${overage(sorted[0]?.overageRate)} above it`
  return `${orList(parts)}${picks}${then}. The fee is billed in advance, overage after each ${per}.`
}

/** The read-only sentence under a charge's fields: what it bills, when, counted by what. */
export function chargeSentence(
  shape: ChargeShape,
  values: ChargeValues,
  ctx: SentenceContext
): string {
  const s = normalizeShape(shape)
  const repeats =
    !!ctx.metricName && !!ctx.unitSingular && singular(ctx.metricName) === singular(ctx.unitSingular)
  const counted = ctx.metricName && !repeats ? ` Counted by the ${ctx.metricName} meter.` : ''
  if (s.source === 'metered' && s.model === 'included') {
    const levels = (values.thresholds as LevelRow[] | undefined) ?? []
    return levels.length ? `${levelsSentence(levels, ctx)}${counted}` : ''
  }
  if (s.source === 'metered' && s.model === 'tiered' && isAllowanceTiers(values.rows)) {
    const [, then] = values.rows as TierRowLike[]
    const per = PER[ctx.term]
    const price = ctx.creditUnitName
      ? `${grouped(then.unitPrice)} ${ctx.creditUnitName}`
      : formatMoney(then.unitPrice, ctx.currency)
    return (
      `${grouped(then.firstUnit)} ${ctx.unitPlural ?? 'units'} a ${per} included, then ` +
      `${price} per ${ctx.unitSingular ?? 'unit'}, billed after each ${per}.${counted}`
    )
  }
  const text = summarizeCharge(
    s,
    { ...values, term: ctx.term },
    {
      currency: ctx.currency,
      metricName: ctx.metricName,
      creditUnitName: ctx.creditUnitName,
      unitSingular: ctx.unitSingular,
    }
  )
  if (!text) return ''
  // Without a unit name the line already reads "per unit of <metric>".
  const meter = s.source === 'metered' && ctx.unitSingular ? counted : ''
  return `${text.charAt(0).toUpperCase()}${text.slice(1)}.${meter}`
}

export type TierMode = 'graduated' | 'volume'

export interface TierLine {
  /**
   * Graduated: the first unit the tier bills, counted from 1. Volume: the least total the tier
   * prices (its first unit as stored, from 0).
   */
  from: number
  /**
   * Graduated: the last unit of the tier. Volume: where the next tier starts, a total the tier
   * no longer prices. None for the last tier.
   */
  to?: number
  units: Decimal
  unitPrice: Decimal
  fee: Decimal
  amount: Decimal
}

const dec = (value: unknown) => {
  try {
    return new Decimal(String(value ?? '').trim() || '0')
  } catch {
    return new Decimal(0)
  }
}

/** What tiers bill for a quantity, as billing-core prices them (pricing/fees.rs). */
export function tierBreakdown(
  rows: TierRowLike[],
  quantity: Decimal.Value,
  mode: TierMode
): { lines: TierLine[]; total: Decimal } {
  const sorted = [...rows].sort((a, b) => Number(a.firstUnit) - Number(b.firstUnit))
  const q = new Decimal(quantity)
  const priced = (row: TierRowLike, units: Decimal, next?: TierRowLike): TierLine => {
    const fee = dec(row.flatFee)
    let amount = units.mul(dec(row.unitPrice)).add(fee)
    if (!isBlank(row.flatCap) && amount.greaterThan(dec(row.flatCap))) amount = dec(row.flatCap)
    return {
      from: Number(row.firstUnit) + (mode === 'graduated' ? 1 : 0),
      to: next ? Number(next.firstUnit) : undefined,
      units,
      unitPrice: dec(row.unitPrice),
      fee,
      amount,
    }
  }

  if (mode === 'volume') {
    const index = sorted.findIndex(
      (row, i) =>
        q.greaterThanOrEqualTo(Number(row.firstUnit)) &&
        (i === sorted.length - 1 || q.lessThan(Number(sorted[i + 1].firstUnit)))
    )
    if (index < 0) return { lines: [], total: new Decimal(0) }
    const line = priced(sorted[index], q, sorted[index + 1])
    return { lines: [line], total: line.amount }
  }

  const lines: TierLine[] = []
  let remaining = q
  sorted.forEach((row, i) => {
    if (remaining.lessThanOrEqualTo(0)) return
    const next = sorted[i + 1]
    const span = next ? new Decimal(Number(next.firstUnit) - Number(row.firstUnit)) : undefined
    const units = span && remaining.greaterThan(span) ? span : remaining
    if (units.greaterThan(0)) lines.push(priced(row, units, next))
    remaining = remaining.sub(units)
  })
  return { lines, total: lines.reduce((sum, line) => sum.add(line.amount), new Decimal(0)) }
}

/**
 * A tier's range as billing reads it (billing-core pricing/fees.rs), from its first unit and the
 * next tier's: graduated tiers bill their own units, "1 to 100" then "101 and above"; a volume
 * tier prices a total from its first unit up to, not including, the next one's: "under 100",
 * "100 to under 500", "500 and above".
 */
export function tierRange(firstUnit: unknown, nextFirstUnit: unknown, mode: TierMode): string {
  const first = Number(toBigInt(firstUnit))
  const next = nextFirstUnit === undefined ? undefined : Number(toBigInt(nextFirstUnit))
  if (mode === 'graduated') {
    return next === undefined
      ? `${grouped(first + 1)} and above`
      : `${grouped(first + 1)} to ${grouped(next)}`
  }
  if (next === undefined) return `${grouped(first)} and above`
  return first === 0 ? `under ${grouped(next)}` : `${grouped(first)} to under ${grouped(next)}`
}

/** Tier rows edited by their last unit: tier i ends where tier i+1 starts. */
export function setTierEnd(rows: TierRowLike[], index: number, lastUnit: unknown): TierRowLike[] {
  if (index >= rows.length - 1) return rows
  return rows.map((row, i) => (i === index + 1 ? { ...row, firstUnit: toBigInt(lastUnit) } : row))
}

/** Splits the last tier: a new one starts after it, the old last tier now ends somewhere. */
export function splitLastTier(rows: TierRowLike[]): TierRowLike[] {
  const last = rows[rows.length - 1]
  const start = last ? toBigInt(last.firstUnit) : BigInt(0)
  const step = start === BigInt(0) ? BigInt(1000) : start
  return [...rows, { firstUnit: start + step, unitPrice: last?.unitPrice ?? '0' }]
}

/**
 * What a customer pays at `quantity` (seats, units of usage), for the result block. A fixed
 * amount ignores the quantity; a model it cannot price returns undefined.
 */
export function customerPays(
  shape: ChargeShape,
  values: ChargeValues,
  quantity: Decimal.Value
): Decimal | undefined {
  const s = normalizeShape(shape)
  const q = new Decimal(quantity)
  switch (s.model) {
    case 'flat':
      return dec(values.rate)
    case 'per_unit': {
      const count = s.source === 'fixed' ? dec(values.quantity ?? 1) : q
      return count.mul(dec(values.unitPrice))
    }
    case 'tiered':
    case 'volume': {
      const rows = (values.rows as TierRowLike[] | undefined) ?? []
      if (rows.length === 0) return undefined
      const count = s.source === 'fixed' ? dec(values.quantity ?? 1) : q
      return tierBreakdown(rows, count, s.model === 'tiered' ? 'graduated' : 'volume').total
    }
    case 'package': {
      const block = dec(values.blockSize)
      if (block.isZero()) return undefined
      const count = s.source === 'fixed' ? dec(values.quantity ?? 1) : q
      return count.div(block).ceil().mul(dec(values.packagePrice))
    }
    case 'percentage':
      return q.mul(dec(values.percent)).div(100)
    case 'dynamic':
      return q
    case 'included': {
      // Several levels: the customer picks one, so no single amount is what they pay.
      const levels = (values.thresholds as LevelRow[] | undefined) ?? []
      const level = levels[0]
      if (!level || levels.length > 1) return undefined
      const over = Decimal.max(q.sub(level.included ?? 0), 0)
      return dec(level.rate).add(over.mul(dec(level.overageRate)))
    }
    default:
      return undefined
  }
}
