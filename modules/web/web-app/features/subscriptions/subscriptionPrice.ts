import Decimal from 'decimal.js'

import { formatPerUnit } from '@/features/billing/metrics/units'
import { formatWalletAmount } from '@/features/credit/format'
import { currencyPrecision } from '@/lib/data/currency-precision'
import { formatCurrency, formatCurrencyNoRounding } from '@/lib/utils/numbers'
import { SubscriptionFeeBillingPeriod } from '@/rpc/api/subscriptions/v1/models_pb'

import type { MetricUnit } from '@/features/billing/metrics/units'
import type {
  SubscriptionFee,
  SubscriptionFee_FeeCreditUnit,
} from '@/rpc/api/subscriptions/v1/models_pb'
import type { CreditUnitLabel } from '@/rpc/api/wallets/v1/models_pb'

export type CreditUnitNames = Pick<CreditUnitLabel, 'name' | 'pluralName' | 'precision'>

/** A subscription component or add-on: an add-on's fee is billed `quantity` times. */
export type PricedFee = {
  fee?: SubscriptionFee
  period: SubscriptionFeeBillingPeriod
  quantity?: number
}

const PERIOD_NOUN: Partial<Record<SubscriptionFeeBillingPeriod, string>> = {
  [SubscriptionFeeBillingPeriod.MONTHLY]: 'month',
  [SubscriptionFeeBillingPeriod.QUARTERLY]: 'quarter',
  [SubscriptionFeeBillingPeriod.SEMIANNUAL]: '6 months',
  [SubscriptionFeeBillingPeriod.YEARLY]: 'year',
}

const PERIOD_ADVERB: Partial<Record<SubscriptionFeeBillingPeriod, string>> = {
  [SubscriptionFeeBillingPeriod.MONTHLY]: 'monthly',
  [SubscriptionFeeBillingPeriod.QUARTERLY]: 'quarterly',
  [SubscriptionFeeBillingPeriod.SEMIANNUAL]: 'every 6 months',
  [SubscriptionFeeBillingPeriod.YEARLY]: 'yearly',
}

const PERIOD_ORDER = [
  SubscriptionFeeBillingPeriod.MONTHLY,
  SubscriptionFeeBillingPeriod.QUARTERLY,
  SubscriptionFeeBillingPeriod.SEMIANNUAL,
  SubscriptionFeeBillingPeriod.YEARLY,
]

export const toMinor = (major: string | Decimal, precision: number): bigint =>
  BigInt(
    new Decimal(major)
      .mul(new Decimal(10).pow(precision))
      .toDecimalPlaces(0, Decimal.ROUND_HALF_UP)
      .toFixed(0)
  )

const parseDecimal = (value: string | undefined): Decimal | undefined => {
  if (!value) return undefined
  try {
    return new Decimal(value)
  } catch {
    return undefined
  }
}

const quantityOf = (fee: PricedFee) => new Decimal(fee.quantity ?? 1)

/** A credit pack granted on the first invoice only, never again. */
const isOncePack = (fee: SubscriptionFee) => fee.fee.case === 'credit' && fee.fee.value.once

/** The unit names of a fee's credit unit, from the customer's wallets, else its code. */
export const creditUnitNames = (
  unit: Pick<SubscriptionFee_FeeCreditUnit, 'creditUnitId' | 'code' | 'precision'>,
  known: Record<string, CreditUnitNames>
): CreditUnitNames =>
  known[unit.creditUnitId] ?? { name: unit.code, pluralName: unit.code, precision: unit.precision }

/** What one billing period of a recurring fee costs, in major units; undefined if not recurring. */
const recurringTotal = (fee: SubscriptionFee, slotCounts: Record<string, number>) => {
  if (isOncePack(fee)) return undefined
  switch (fee.fee.case) {
    case 'rate':
      return parseDecimal(fee.fee.value.rate)
    case 'recurring':
      return (
        parseDecimal(fee.fee.value.total) ??
        parseDecimal(fee.fee.value.rate)?.mul(fee.fee.value.quantity)
      )
    case 'capacity':
      return parseDecimal(fee.fee.value.rate)
    case 'slot':
      return parseDecimal(fee.fee.value.unitRate)?.mul(
        slotCounts[fee.fee.value.unit] ?? fee.fee.value.initialSlots
      )
    case 'credit':
      return parseDecimal(fee.fee.value.total)
    default:
      return undefined
  }
}

/** The credit a pack grants per billing period (`once`: on the first invoice only), keyed by credit unit id or currency. */
export const packCredits = (
  fees: PricedFee[],
  currency: string
): {
  key: string
  creditUnit?: SubscriptionFee_FeeCreditUnit
  period: SubscriptionFeeBillingPeriod
  once: boolean
  total: Decimal
}[] =>
  fees.flatMap(f => {
    const fee = f.fee
    if (!fee || fee.fee.case !== 'credit') return []
    const total = parseDecimal(fee.fee.value.creditTotal)
    if (!total) return []
    const creditUnit = fee.creditUnit
    return [
      {
        key: creditUnit?.creditUnitId ?? currency,
        creditUnit,
        period: f.period,
        once: fee.fee.value.once,
        total: total.mul(quantityOf(f)),
      },
    ]
  })

const usageFees = (fees: PricedFee[]) =>
  fees.flatMap(f => {
    const fee = f.fee
    if (!fee || fee.fee.case !== 'usage') return []
    return [{ period: f.period, fee, usage: fee.fee.value }]
  })

const joinAnd = (items: string[]) =>
  items.length <= 1
    ? (items[0] ?? '')
    : `${items.slice(0, -1).join(', ')} and ${items[items.length - 1]}`

export type PriceSummaryInput = {
  currency: string
  fees: PricedFee[]
  minimumSubunit?: bigint
  slotCounts?: Record<string, number>
  creditUnits?: Record<string, CreditUnitNames>
  /** Usage is refused once the customer's credit runs out: paid ahead from credit. */
  prepaid?: boolean
}

export type PriceSummary = { headline: string; details: string[] }

/** "$500.00 / month" · "includes $550.00 of credit" · "usage beyond credit billed monthly". */
export const summarizeSubscriptionPrice = ({
  currency,
  fees,
  minimumSubunit,
  slotCounts = {},
  creditUnits = {},
  prepaid = false,
}: PriceSummaryInput): PriceSummary => {
  const precision = currencyPrecision(currency)
  const money = (major: Decimal) => formatCurrency(toMinor(major, precision), currency)

  const perPeriod = new Map<SubscriptionFeeBillingPeriod, Decimal>()
  for (const f of fees) {
    if (!f.fee || f.period === SubscriptionFeeBillingPeriod.ONE_TIME) continue
    const total = recurringTotal(f.fee, slotCounts)
    if (!total || total.isZero()) continue
    perPeriod.set(
      f.period,
      (perPeriod.get(f.period) ?? new Decimal(0)).add(total.mul(quantityOf(f)))
    )
  }
  const recurring = PERIOD_ORDER.flatMap(period => {
    const total = perPeriod.get(period)
    return total ? [`${money(total)} / ${PERIOD_NOUN[period]}`] : []
  })

  const creditText = (creditUnit: SubscriptionFee_FeeCreditUnit | undefined, total: Decimal) => {
    if (!creditUnit) return `${money(total)} of credit`
    const names = creditUnitNames(creditUnit, creditUnits)
    return formatWalletAmount(toMinor(total, names.precision), {
      currency: creditUnit.code,
      creditUnit: names,
    })
  }
  const creditTotals = (once: boolean) => {
    const groups = new Map<string, { creditUnit?: SubscriptionFee_FeeCreditUnit; total: Decimal }>()
    for (const pack of packCredits(fees, currency).filter(p => p.once === once)) {
      const group = groups.get(pack.key)
      groups.set(pack.key, {
        creditUnit: pack.creditUnit,
        total: (group?.total ?? new Decimal(0)).add(pack.total),
      })
    }
    return [...groups.values()]
      .filter(g => !g.total.isZero())
      .map(({ creditUnit, total }) => creditText(creditUnit, total))
  }
  const credits = creditTotals(false)
  const onceCredits = creditTotals(true)

  const usage = usageFees(fees)
  const usageCadences = PERIOD_ORDER.filter(p => usage.some(u => u.period === p)).map(
    p => PERIOD_ADVERB[p] as string
  )
  const usageCadence = joinAnd(usageCadences)
  // The minimum applies per billing period, which the usage (else recurring) fees set.
  const minimumPeriod = usage[0]?.period ?? PERIOD_ORDER.find(p => perPeriod.has(p))
  const minimumPer = minimumPeriod !== undefined ? PERIOD_NOUN[minimumPeriod] : undefined
  const minimum =
    minimumSubunit && minimumSubunit > 0n
      ? minimumPer
        ? `${formatCurrency(minimumSubunit, currency)} / ${minimumPer} minimum`
        : `${formatCurrency(minimumSubunit, currency)} minimum per period`
      : undefined
  const includes = [
    ...(credits.length > 0 ? [`includes ${joinAnd(credits)}`] : []),
    ...(onceCredits.length > 0 ? [`plus ${joinAnd(onceCredits)} once at start`] : []),
  ]

  if (recurring.length > 0) {
    return {
      headline: recurring.join(' + '),
      details: [
        ...(minimum ? [minimum] : []),
        ...includes,
        ...(usage.length > 0
          ? [
              credits.length > 0
                ? `usage beyond credit billed ${usageCadence}`
                : `plus usage billed ${usageCadence}`,
            ]
          : []),
      ],
    }
  }
  if (minimum) {
    return {
      headline: minimum,
      details: [...includes, ...(usage.length > 0 ? ['usage at contract rates'] : [])],
    }
  }
  if (usage.length > 0) {
    return prepaid
      ? { headline: 'Prepaid credit', details: [...includes, 'usage billed from credit'] }
      : { headline: 'Usage-based', details: [...includes, `billed ${usageCadence}`] }
  }
  if (fees.some(f => f.fee?.fee.case === 'oneTime')) {
    return { headline: 'One-time charges only', details: includes }
  }
  return { headline: 'No recurring charges', details: includes }
}

/** Every per-unit price a usage fee can charge, in its pricing's denomination. */
const usageUnitPrices = (fee: SubscriptionFee): Decimal[] => {
  if (fee.fee.case !== 'usage') return []
  const model = fee.fee.value.model
  switch (model.case) {
    case 'perUnit':
      return [parseDecimal(model.value)].filter((d): d is Decimal => !!d)
    case 'tiered':
    case 'volume':
      return model.value.rows.map(r => parseDecimal(r.unitPrice)).filter((d): d is Decimal => !!d)
    case 'matrix':
      return model.value.rows
        .map(r => parseDecimal(r.perUnitPrice))
        .filter((d): d is Decimal => !!d)
    case 'package': {
      const price = parseDecimal(model.value.packagePrice)
      return price && model.value.blockSize > 0n
        ? [price.div(model.value.blockSize.toString())]
        : []
    }
    default:
      return []
  }
}

const formatCredits = (value: Decimal, precision: number) =>
  new Intl.NumberFormat(undefined, { maximumFractionDigits: Math.max(precision, 6) }).format(
    value.toNumber()
  )

/**
 * The range of usage prices, one entry per unit and denomination: "$2.00 – $60.00 per 1M units".
 * Undefined while a usage fee's metric unit is unknown, so a converted price never shows per unit.
 */
export const usagePriceRanges = ({
  currency,
  fees,
  units,
  creditUnits = {},
}: {
  currency: string
  fees: PricedFee[]
  units: Record<string, MetricUnit | undefined>
  creditUnits?: Record<string, CreditUnitNames>
}): string[] | undefined => {
  const groups = new Map<
    string,
    { unit: MetricUnit; creditUnit?: SubscriptionFee_FeeCreditUnit; min: Decimal; max: Decimal }
  >()
  for (const { usage, fee } of usageFees(fees)) {
    const unit = units[usage.metricId]
    if (!unit) return undefined
    const prices = usageUnitPrices(fee)
    if (prices.length === 0) continue
    const creditUnit = fee.creditUnit?.rate ? fee.creditUnit : undefined
    const key = `${creditUnit?.creditUnitId ?? currency}|${formatPerUnit(unit)}`
    const min = Decimal.min(...prices)
    const max = Decimal.max(...prices)
    const group = groups.get(key)
    groups.set(key, {
      unit,
      creditUnit,
      min: group ? Decimal.min(group.min, min) : min,
      max: group ? Decimal.max(group.max, max) : max,
    })
  }
  return [...groups.values()].map(({ unit, creditUnit, min, max }) => {
    const per = formatPerUnit(unit)
    if (creditUnit) {
      const names = creditUnitNames(creditUnit, creditUnits)
      const range = min.eq(max)
        ? formatCredits(min, names.precision)
        : `${formatCredits(min, names.precision)} – ${formatCredits(max, names.precision)}`
      return `${range} ${min.eq(1) && max.eq(1) ? names.name : names.pluralName} ${per}`
    }
    const price = (d: Decimal) => formatCurrencyNoRounding(d.toString(), currency)
    return min.eq(max) ? `${price(min)} ${per}` : `${price(min)} – ${price(max)} ${per}`
  })
}

/** Credit the recurring packs grant per month for `key` (credit unit id or currency); undefined unless all are monthly. */
export const monthlyPackCredit = (fees: PricedFee[], currency: string, key: string) => {
  const packs = packCredits(fees, currency).filter(p => p.key === key && !p.once)
  if (packs.length === 0) return undefined
  if (packs.some(p => p.period !== SubscriptionFeeBillingPeriod.MONTHLY)) return undefined
  return packs.reduce((sum, p) => sum.add(p.total), new Decimal(0))
}
