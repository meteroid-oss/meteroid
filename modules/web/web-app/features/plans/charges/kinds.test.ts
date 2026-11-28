import { describe, expect, it } from 'vitest'

import {
  addLevel,
  allowanceRows,
  chargeSentence,
  customerPays,
  includedUsage,
  kindOf,
  kindOffered,
  overageVaries,
  percentOf,
  pickKind,
  setTierEnd,
  shapeForKind,
  splitLastTier,
  tierBreakdown,
  tierRange,
  unitIsCurrency,
  valuesForKind,
  withIncludedUsage,
} from './kinds'
import { hasDuplicateLevels } from './schema'
import { formatMoney } from './summary'

import type { ChargeShape } from './compile'

const usageTiers: ChargeShape = {
  source: 'metered',
  model: 'tiered',
  recurrence: 'recurring',
  billing: 'arrears',
}
const fixed = shapeForKind('fixed') as ChargeShape

describe('kinds', () => {
  it('reads usage as usage, some included or not', () => {
    expect(kindOf(usageTiers)).toBe('usage')
    expect(kindOf(shapeForKind('usage') as ChargeShape)).toBe('usage')
  })

  it('reads a one-time fee as a fixed fee, and seats as seats', () => {
    const once: ChargeShape = { ...fixed, recurrence: 'one_time' }
    expect(kindOf(once)).toBe('fixed')
    expect(kindOf(shapeForKind('seat') as ChargeShape)).toBe('seat')
    expect(kindOf({ ...fixed, model: 'credit_pack' })).toBe('credit')
  })

  it("offers only the kinds a product's fee type allows", () => {
    const rate = { feeType: 'rate' as const }
    expect(kindOffered('fixed', rate)).toBe(true)
    expect(shapeForKind('fixed', rate)?.recurrence).toBe('recurring')
    expect(kindOffered('usage', rate)).toBe(false)
    expect(kindOffered('included', rate)).toBe(true)
  })

  it('sells a one-time product as a fixed fee charged once', () => {
    const oneTime = { feeType: 'oneTime' as const }
    expect(kindOffered('fixed', oneTime)).toBe(true)
    expect(shapeForKind('fixed', oneTime)).toEqual({
      source: 'fixed',
      model: 'flat',
      recurrence: 'one_time',
      billing: 'advance',
    })
    expect(shapeForKind('fixed')?.recurrence).toBe('recurring')
  })

  it('never makes a fixed fee of a credit product', () => {
    expect(kindOffered('fixed', { feeType: 'credit' })).toBe(false)
    expect(shapeForKind('credit', { feeType: 'credit' })?.model).toBe('credit_pack')
  })

  it('lets a per-unit usage charge become tiered, keeping its metric', () => {
    const usage = { feeType: 'usage' as const }
    const perUnit = shapeForKind('usage', usage) as ChargeShape
    expect(perUnit.model).toBe('per_unit')
    const values = valuesForKind('usage', usageTiers, { metricId: 'm', unitPrice: '0.002' }, perUnit)
    expect(values.metricId).toBe('m')
    expect(Array.isArray(values.rows)).toBe(true)
  })

  it('sells a capacity product in levels, and only it', () => {
    const capacity = { feeType: 'capacity' as const }
    expect(shapeForKind('levels', capacity)?.model).toBe('included')
    expect(shapeForKind('levels')?.model).toBe('included')
    expect(kindOffered('usage', capacity)).toBe(false)
    expect(kindOffered('levels', { feeType: 'usage' })).toBe(false)
  })

  it('reads a capacity charge as usage levels', () => {
    expect(kindOf(shapeForKind('levels') as ChargeShape)).toBe('levels')
  })

  it('turns an allowance into a first level, and a level back into an allowance', () => {
    const levels = shapeForKind('levels') as ChargeShape
    const fromAllowance = valuesForKind(
      'levels',
      levels,
      { rows: allowanceRows(125_000, '0.005') },
      usageTiers
    )
    expect(fromAllowance.thresholds).toEqual([
      { included: 125_000, rate: '0.00', overageRate: '0.005' },
    ])
    const back = pickKind(
      'usage',
      undefined,
      { thresholds: [{ included: 50_000, rate: '479', overageRate: '0.005' }] },
      levels
    )
    expect(back?.shape.model).toBe('tiered')
    expect(back?.values.rows).toEqual(allowanceRows(50_000, '0.005'))
  })

  it('turns a level including nothing into plain usage at its overage', () => {
    const levels = shapeForKind('levels') as ChargeShape
    const back = pickKind(
      'usage',
      undefined,
      { thresholds: [{ included: 0, rate: '0.00', overageRate: '0.004' }] },
      levels
    )
    expect(back?.shape.model).toBe('per_unit')
    expect(back?.values.unitPrice).toBe('0.004')
  })

  it('refuses a kind the product cannot be sold as', () => {
    expect(pickKind('seat', { feeType: 'usage' }, {})).toBeUndefined()
  })
})

describe('usage with some included', () => {
  const perUnit = shapeForKind('usage') as ChargeShape

  it('reads nothing included on plain usage and on other tiers', () => {
    expect(includedUsage(perUnit, { unitPrice: '0.002' })).toBe(BigInt(0))
    expect(
      includedUsage(usageTiers, {
        rows: [
          { firstUnit: BigInt(0), unitPrice: '0.001' },
          { firstUnit: BigInt(10), unitPrice: '0.0005' },
        ],
      })
    ).toBe(BigInt(0))
    expect(includedUsage(usageTiers, { rows: allowanceRows(1_000_000, '0.0005') })).toBe(
      BigInt(1_000_000)
    )
  })

  it('stores an included amount as a free first tier, then the price per unit', () => {
    const next = withIncludedUsage(perUnit, { metricId: 'm', unitPrice: '0.002' }, '1,000')
    expect(next.shape).toEqual({ ...perUnit, model: 'tiered' })
    expect(next.values.rows).toEqual(allowanceRows(1000, '0.002'))
    expect(next.values.metricId).toBe('m')
  })

  it('changes the included amount, keeping the price after it', () => {
    const next = withIncludedUsage(usageTiers, { rows: allowanceRows(1000, '0.002') }, 5000)
    expect(next.values.rows).toEqual(allowanceRows(5000, '0.002'))
  })

  it('stores nothing included as plain usage per unit, at the price after the free tier', () => {
    const next = withIncludedUsage(usageTiers, { rows: allowanceRows(1000, '0.002') }, '')
    expect(next.shape).toEqual({ ...usageTiers, model: 'per_unit' })
    expect(next.values.unitPrice).toBe('0.002')
  })
})

describe('chargeSentence', () => {
  it('writes an allowance out, with its meter', () => {
    const sentence = chargeSentence(
      usageTiers,
      { rows: allowanceRows(1_000_000, '0.0005') },
      {
        currency: 'USD',
        term: 'MONTHLY',
        metricName: 'API calls',
        unitSingular: 'call',
        unitPlural: 'calls',
      }
    )
    expect(sentence).toBe(
      `1,000,000 calls a month included, then ${formatMoney('0.0005', 'USD')} per call, billed after each month. Counted by the API calls meter.`
    )
  })

  it('reads a fixed amount on the selected term', () => {
    expect(chargeSentence(fixed, { rate: '149' }, { currency: 'USD', term: 'ANNUAL' })).toBe(
      `${formatMoney('149', 'USD')} per year, billed in advance.`
    )
  })

  it('prices usage per the unit the metric names, the meter said once', () => {
    const perUnit = shapeForKind('usage') as ChargeShape
    const price = formatMoney('0.03', 'EUR')
    const ctx = { currency: 'EUR', term: 'MONTHLY' as const, metricName: 'AI Assist Actions' }
    expect(chargeSentence(perUnit, { unitPrice: '0.03' }, { ...ctx, unitSingular: 'AI Assist Action' })).toBe(
      `${price} per AI Assist Action, billed monthly in arrears.`
    )
    expect(chargeSentence(perUnit, { unitPrice: '0.03' }, ctx)).toBe(
      `${price} per unit of AI Assist Actions, billed monthly in arrears.`
    )
  })
})

describe('levels', () => {
  const levels = shapeForKind('levels') as ChargeShape
  const plans = [
    { included: 125_000, rate: '899', overageRate: '0.005' },
    { included: 50_000, rate: '479', overageRate: '0.005' },
  ]

  it('reads one level as a fee, its allowance and the overage above it', () => {
    const sentence = chargeSentence(
      levels,
      { thresholds: [{ included: 125_000, rate: '479', overageRate: '0.01' }] },
      {
        currency: 'USD',
        term: 'MONTHLY',
        metricName: 'API calls',
        unitSingular: 'call',
        unitPlural: 'calls',
      }
    )
    expect(sentence).toBe(
      `${formatMoney('479', 'USD')} a month for up to 125K calls, then ` +
        `${formatMoney('0.01', 'USD')} per call above it. The fee is billed in advance, ` +
        'overage after each month. Counted by the API calls meter.'
    )
  })

  it("reads several levels smallest first, a money metric's overage as a percentage", () => {
    const sentence = chargeSentence(
      levels,
      { thresholds: plans },
      {
        currency: 'EUR',
        term: 'MONTHLY',
        metricName: 'Managed revenue',
        unitSingular: 'euro',
        unitPlural: 'euros',
        moneyUnit: true,
      }
    )
    expect(sentence).toBe(
      `${formatMoney('479', 'EUR')} a month for up to €50K or ${formatMoney('899', 'EUR')} for ` +
        'up to €125K, as the customer picks, then 0.5% above it. The fee is billed in advance, ' +
        'overage after each month. Counted by the Managed revenue meter.'
    )
  })

  it('gives each level its own overage when they differ', () => {
    const own = [plans[1], { ...plans[0], overageRate: '0.004' }]
    expect(overageVaries(own)).toBe(true)
    const ctx = { currency: 'EUR', term: 'MONTHLY' as const }
    const sentence = chargeSentence(levels, { thresholds: own }, ctx)
    const then = (rate: string) => `(then ${formatMoney(rate, 'EUR')} per unit)`
    expect(sentence).toContain(`for up to 50K units ${then('0.005')}`)
    expect(sentence).toContain(`for up to 125K units ${then('0.004')}`)
    expect(sentence).not.toContain('above it')
  })

  it('says one overage once when every level has the same', () => {
    expect(overageVaries(plans)).toBe(false)
    expect(overageVaries([plans[0]])).toBe(false)
    expect(overageVaries([plans[0], { ...plans[1], overageRate: '0.0050' }])).toBe(false)
    const sentence = chargeSentence(levels, { thresholds: plans }, { currency: 'EUR', term: 'MONTHLY' })
    expect(sentence).toContain(`, as the customer picks, then ${formatMoney('0.005', 'EUR')} per unit above it.`)
    expect(sentence).not.toContain('(then')
  })

  it('adds a level after the largest, at its price and overage', () => {
    const added = addLevel(plans)
    expect(added).toHaveLength(3)
    expect(added[2]).toEqual({ included: 250_000, rate: '899', overageRate: '0.005' })
    expect(addLevel([{ included: 0, rate: '0', overageRate: '0' }])[1].included).toBe(1000)
  })

  it("starts a new level on the overage of the level before it, each kept its own", () => {
    const own = [
      { included: 50_000, rate: '479', overageRate: '0.006' },
      { included: 125_000, rate: '899', overageRate: '0.004' },
    ]
    const added = addLevel(own)
    expect(added.map(level => level.overageRate)).toEqual(['0.006', '0.004', '0.004'])
    expect(overageVaries(added)).toBe(true)
    expect(addLevel([])[0]).toEqual({ included: 1000, rate: '0.00', overageRate: '0.00000000' })
  })

  it('refuses two levels including the same amount', () => {
    expect(hasDuplicateLevels(plans)).toBe(false)
    expect(hasDuplicateLevels([...plans, { ...plans[0], rate: '999' }])).toBe(true)
  })

  it("prices one level; several are the customer's pick", () => {
    expect(customerPays(levels, { thresholds: [plans[1]] }, 60_000)?.toNumber()).toBe(529)
    expect(customerPays(levels, { thresholds: plans }, 60_000)).toBeUndefined()
  })

  it('reads a metric as money only when its unit names the currency', () => {
    expect(unitIsCurrency({ singular: 'euro', plural: 'euros' }, 'EUR')).toBe(true)
    expect(unitIsCurrency({ singular: 'EUR', plural: 'EUR' }, 'eur')).toBe(true)
    expect(unitIsCurrency({ singular: '€' }, 'EUR')).toBe(true)
    expect(unitIsCurrency({ singular: 'euro', plural: 'euros' }, 'USD')).toBe(false)
    expect(unitIsCurrency({ singular: 'euro', conversionFactor: 1000 }, 'EUR')).toBe(false)
    expect(unitIsCurrency({ singular: 'call', plural: 'calls' }, 'EUR')).toBe(false)
    expect(unitIsCurrency({}, 'EUR')).toBe(false)
    expect(unitIsCurrency(undefined, 'EUR')).toBe(false)
  })

  it('reads a rate per unit of money as a percentage', () => {
    expect(percentOf('0.005')).toBe('0.5%')
    expect(percentOf('0.00500000')).toBe('0.5%')
    expect(percentOf('0.0125')).toBe('1.25%')
  })
})

describe('tiers', () => {
  const rows = [
    { firstUnit: BigInt(0), unitPrice: '0' },
    { firstUnit: BigInt(1_000_000), unitPrice: '0.0004', flatFee: '5.00' },
  ]

  it('graduated: each tier bills its own units, a fee once entered', () => {
    const { lines, total } = tierBreakdown(rows, 3_500_000, 'graduated')
    expect(lines.map(l => [l.from, l.to, l.units.toNumber(), l.amount.toNumber()])).toEqual([
      [1, 1_000_000, 1_000_000, 0],
      [1_000_001, undefined, 2_500_000, 1005],
    ])
    expect(total.toNumber()).toBe(1005)
  })

  it('volume: every unit at the tier reached', () => {
    expect(tierBreakdown(rows, 3_500_000, 'volume').total.toNumber()).toBe(1405)
  })

  it("volume: a total equal to the next tier's start is priced there (billing-core)", () => {
    const volume = [
      { firstUnit: BigInt(0), unitPrice: '1' },
      { firstUnit: BigInt(100), unitPrice: '0.5' },
    ]
    expect(tierBreakdown(volume, 99, 'volume').total.toNumber()).toBe(99)
    const at = tierBreakdown(volume, 100, 'volume')
    expect(at.total.toNumber()).toBe(50)
    expect(at.lines.map(l => [l.from, l.to])).toEqual([[100, undefined]])
    expect(tierBreakdown(volume, 50, 'volume').lines.map(l => [l.from, l.to])).toEqual([[0, 100]])
  })

  it('labels tiers as each model bills them', () => {
    expect(tierRange(0, 100, 'graduated')).toBe('1 to 100')
    expect(tierRange(100, undefined, 'graduated')).toBe('101 and above')
    expect(tierRange(0, 100, 'volume')).toBe('under 100')
    expect(tierRange(100, 500, 'volume')).toBe('100 to under 500')
    expect(tierRange(BigInt(500), undefined, 'volume')).toBe('500 and above')
  })

  it('a cap bounds what a tier costs', () => {
    const capped = [rows[0], { ...rows[1], flatCap: '500' }]
    expect(tierBreakdown(capped, 3_500_000, 'graduated').total.toNumber()).toBe(500)
  })

  it('edits a tier by its last unit and splits the last one', () => {
    expect(setTierEnd(rows, 0, '2000000')[1].firstUnit).toBe(BigInt(2_000_000))
    const split = splitLastTier(rows)
    expect(split).toHaveLength(3)
    expect(split[2].firstUnit).toBe(BigInt(2_000_000))
  })
})

describe('customerPays', () => {
  it('prices usage over the included amount', () => {
    expect(
      customerPays(usageTiers, { rows: allowanceRows(1_000_000, '0.0005') }, 2_400_000)?.toNumber()
    ).toBe(700)
  })

  it('prices seats by the count typed', () => {
    const seats = shapeForKind('seat') as ChargeShape
    expect(customerPays(seats, { unitPrice: '12' }, 10)?.toNumber()).toBe(120)
  })

  it('a fixed amount ignores the quantity', () => {
    expect(customerPays(fixed, { rate: '149' }, 999)?.toNumber()).toBe(149)
  })

  it('a percentage is its share of the amount, a refund crediting it back', () => {
    const percentage: ChargeShape = { ...usageTiers, model: 'percentage' }
    expect(customerPays(percentage, { percent: '2.9' }, 1234.56)?.toString()).toBe('35.80224')
    expect(customerPays(percentage, { percent: '2' }, -50)?.toNumber()).toBe(-1)
  })

  it('a price from events is the amount itself', () => {
    const dynamic: ChargeShape = { ...usageTiers, model: 'dynamic' }
    expect(customerPays(dynamic, {}, 1234.56)?.toString()).toBe('1234.56')
  })
})
