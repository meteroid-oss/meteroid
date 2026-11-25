import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { buildPriceInputs } from '@/features/pricing/conversions'
import { formDataToProtoPricing, protoPricingToFormData } from '@/features/pricing/mapping'
import { UsagePricingSchema } from '@/rpc/api/prices/v1/models_pb'

import {
  PRESETS,
  bestFit,
  chargeDefaults,
  compileCharge,
  decompileCharge,
  normalizeShape,
  presetOf,
  pricesKeptOnEdit,
  shapeFits,
  valuesForShape,
} from './compile'
import { summarizeCharge } from './summary'

import type { ChargeShape } from './compile'

const preset = (id: string) => PRESETS.find(p => p.id === id)!.shape as ChargeShape

const rows = [
  { firstUnit: BigInt(0), unitPrice: '10.00' },
  { firstUnit: BigInt(10), unitPrice: '8.00' },
]

describe('compileCharge', () => {
  it('a flat fee in advance is a rate', () => {
    const compiled = compileCharge(preset('platform'), { term: 'ANNUAL', rate: '99.00' })
    expect(compiled.feeType).toBe('rate')
    expect(compiled.pricingType).toBe('rate')
    expect(compiled.formData).toEqual({ term: 'ANNUAL', rate: '99.00' })
  })

  it('a flat fee in arrears is an extra recurring fee of one unit', () => {
    const shape: ChargeShape = { ...preset('platform'), billing: 'arrears' }
    const compiled = compileCharge(shape, { rate: '20.00' })
    expect(compiled.feeType).toBe('extraRecurring')
    expect(compiled.formData).toMatchObject({
      term: 'MONTHLY',
      billingType: 'ARREAR',
      unitPrice: '20.00',
      quantity: 1,
      quantityModel: 'per_unit',
    })
  })

  it('a fixed count per unit is unit price × quantity, with no model', () => {
    const shape: ChargeShape = {
      source: 'fixed',
      model: 'per_unit',
      recurrence: 'recurring',
      billing: 'advance',
    }
    const compiled = compileCharge(shape, { unitPrice: '5.00', quantity: 3 })
    expect(compiled.feeType).toBe('extraRecurring')
    expect(compiled.formData).toMatchObject({ billingType: 'ADVANCE', unitPrice: '5.00', quantity: 3 })
    const pricing = formDataToProtoPricing(compiled.pricingType, compiled.formData)
    expect(pricing.case).toBe('extraRecurringPricing')
    if (pricing.case !== 'extraRecurringPricing') return
    expect(pricing.value.model).toBeUndefined()
    expect(pricing.value.unitPrice).toBe('5.00')
  })

  it('a one-time fixed count can be priced on tiers, and reads back', () => {
    const shape: ChargeShape = {
      source: 'fixed',
      model: 'tiered',
      recurrence: 'one_time',
      billing: 'advance',
    }
    const compiled = compileCharge(shape, { quantity: 12, rows })
    expect(compiled.feeType).toBe('oneTime')
    expect(compiled.pricingType).toBe('oneTime')
    const pricing = formDataToProtoPricing(compiled.pricingType, compiled.formData)
    expect(pricing.case).toBe('oneTimePricing')
    if (pricing.case !== 'oneTimePricing') return
    expect(pricing.value.quantity).toBe(12)
    expect(pricing.value.unitPrice).toBe('0')
    expect(pricing.value.model?.model.case).toBe('tiered')

    const form = protoPricingToFormData(pricing)
    expect(form?.data.quantityModel).toBe('tiered')
    expect((form?.data.rows as unknown[]).length).toBe(2)
    expect(decompileCharge('oneTime', form!.data).shape).toEqual(shape)
  })

  it('seats compile to a slot fee with their model', () => {
    const perSeat = compileCharge(preset('seat'), { unitPrice: '12.00', slotUnitName: 'Editors' })
    expect(perSeat.feeType).toBe('slot')
    expect(perSeat.formData).toMatchObject({
      seatModel: 'per_unit',
      unitRate: '12.00',
      slotUnitName: 'Editors',
    })

    const tiered = compileCharge({ ...preset('seat'), model: 'tiered' }, { rows, minSlots: 3 })
    expect(tiered.formData).toMatchObject({ seatModel: 'tiered', rows, minSlots: 3 })
    expect(tiered.formData.unitRate).toBeUndefined()
  })

  it('usage by dimension from a rate card compiles to a usage fee naming the card', () => {
    const compiled = compileCharge(
      { ...preset('usage'), model: 'by_dimension' },
      {
        metricId: 'm1',
        rateSource: 'card',
        rateCardId: 'rc1',
        markup: '-20',
        unitPrice: 'stale',
        keyRates: [{ key: ['a'], rate: '1' }],
      }
    )
    expect(compiled.feeType).toBe('usage')
    expect(compiled.pricingType).toBe('rateCard')
    expect(compiled.formData).toMatchObject({
      metricId: 'm1',
      usageModel: 'rate_card',
      rateCardId: 'rc1',
      markup: '-20',
    })
    expect(compiled.formData).not.toHaveProperty('unitPrice')
    expect(compiled.formData).not.toHaveProperty('keyRates')
    const pricing = formDataToProtoPricing(compiled.pricingType, compiled.formData)
    expect(pricing.case === 'usagePricing' && pricing.value.model.case).toBe('rateCard')
    expect(decompileCharge(compiled.feeType, compiled.formData)).toMatchObject({
      shape: { source: 'metered', model: 'by_dimension' },
      values: { rateSource: 'card' },
    })
  })

  it('rates in this plan compile to rates by dimension, an empty cell matching any value', () => {
    const compiled = compileCharge(
      { ...preset('usage'), model: 'by_dimension' },
      {
        metricId: 'm1',
        rateSource: 'plan',
        dimensions: ['model', 'token_type'],
        keyRates: [
          { key: ['gpt-*', 'input'], rate: '2.5' },
          { key: ['gpt-5', ''], rate: '10' },
        ],
        unitSize: '1000000',
        onUnmatched: 'ignore',
        rateCardId: 'stale',
      }
    )
    expect(compiled.feeType).toBe('usage')
    expect(compiled.pricingType).toBe('dimensionRates')
    expect(compiled.formData).toMatchObject({ usageModel: 'dimension_rates' })
    expect(compiled.formData).not.toHaveProperty('rateCardId')

    const pricing = formDataToProtoPricing(compiled.pricingType, compiled.formData)
    expect(pricing.case).toBe('usagePricing')
    if (pricing.case !== 'usagePricing' || pricing.value.model.case !== 'dimensionRates') return
    const rates = pricing.value.model.value
    expect(rates.dimensions).toEqual(['model', 'token_type'])
    expect(rates.rates.map(rate => [rate.key, rate.rate])).toEqual([
      [['gpt-*', 'input'], '2.5'],
      [['gpt-5', '*'], '10'],
    ])
    expect(rates.unitSize).toBe('1000000')

    // Read back, it is the same charge.
    const form = protoPricingToFormData(pricing)
    expect(form?.pricingType).toBe('dimensionRates')
    const { shape, values } = decompileCharge('usage', { ...form!.data, usageModel: 'dimension_rates' })
    expect(shape.model).toBe('by_dimension')
    expect(values).toMatchObject({
      rateSource: 'plan',
      dimensions: ['model', 'token_type'],
      unitSize: '1000000',
      onUnmatched: 'ignore',
    })
  })

  it('a matrix stored before reads as rates in this plan', () => {
    const form = protoPricingToFormData({
      case: 'usagePricing',
      value: create(UsagePricingSchema, {
        model: {
          case: 'matrix',
          value: {
            rows: [
              {
                perUnitPrice: '2',
                dimension1: { key: 'region', value: 'eu' },
                dimension2: { key: 'tier', value: 'gold' },
              },
              { perUnitPrice: '1', dimension1: { key: 'region', value: 'us' } },
            ],
          },
        },
      }),
    })
    expect(form?.pricingType).toBe('dimensionRates')
    expect(form?.data).toMatchObject({
      dimensions: ['region', 'tier'],
      keyRates: [
        { key: ['eu', 'gold'], rate: '2' },
        { key: ['us', '*'], rate: '1' },
      ],
      unitSize: '1',
      onUnmatched: 'ignore',
    })
    expect(decompileCharge('usage', { ...form!.data, usageModel: 'matrix' })).toMatchObject({
      shape: { model: 'by_dimension' },
      values: { rateSource: 'plan' },
    })
  })

  it('metered with an included amount is a capacity fee', () => {
    const thresholds = [{ included: 1000, rate: '50.00', overageRate: '0.01' }]
    const compiled = compileCharge(preset('usage_included'), { metricId: 'm1', thresholds })
    expect(compiled.feeType).toBe('capacity')
    expect(compiled.formData).toMatchObject({ metricId: 'm1', thresholds })
  })

  it('every level of a capacity fee becomes a price of its cell', () => {
    const thresholds = [
      { included: 50_000, rate: '479', overageRate: '0.005' },
      { included: 125_000, rate: '899', overageRate: '0.005' },
      { included: 250_000, rate: '1499', overageRate: '0.004' },
    ]
    const compiled = compileCharge(preset('usage_included'), {
      term: 'MONTHLY',
      metricId: 'm1',
      thresholds,
    })
    const inputs = buildPriceInputs(compiled.pricingType, compiled.formData, 'EUR')
    expect(inputs).toHaveLength(3)
    expect(
      inputs.map(input => {
        const pricing = input.pricing
        return pricing.case === 'capacityPricing'
          ? [Number(pricing.value.included), pricing.value.rate, pricing.value.overageRate]
          : undefined
      })
    ).toEqual(thresholds.map(level => [level.included, level.rate, level.overageRate]))
    expect(new Set(inputs.map(input => input.cadence)).size).toBe(1)
  })

  it('a credit pack granted once is a credit fee with once set', () => {
    const shape: ChargeShape = { ...preset('credit'), recurrence: 'one_time' }
    const compiled = compileCharge(shape, { ...chargeDefaults(shape), creditAmount: '100' })
    expect(compiled.feeType).toBe('credit')
    expect(compiled.formData).toMatchObject({ once: true, creditAmount: '100' })
  })
})

describe('valid shapes', () => {
  it('invalid combinations are normalized away', () => {
    expect(
      normalizeShape({ source: 'seats', model: 'flat', recurrence: 'one_time', billing: 'arrears' })
    ).toEqual({ source: 'seats', model: 'per_unit', recurrence: 'recurring', billing: 'advance' })
    expect(
      normalizeShape({ source: 'metered', model: 'flat', recurrence: 'recurring', billing: 'advance' })
    ).toEqual({ source: 'metered', model: 'per_unit', recurrence: 'recurring', billing: 'arrears' })
    expect(
      normalizeShape({ source: 'fixed', model: 'by_dimension', recurrence: 'recurring', billing: 'advance' })
        .model
    ).toBe('flat')
  })

  it('a locked product only takes shapes of its fee type', () => {
    const rateLock = { feeType: 'rate' as const }
    expect(shapeFits(preset('platform'), rateLock)).toBe(true)
    expect(shapeFits({ ...preset('platform'), billing: 'arrears' }, rateLock)).toBe(false)
    expect(bestFit(preset('platform'), { source: 'seats' }, rateLock)).toBeUndefined()

    const erLock = { feeType: 'extraRecurring' as const, billingType: 'ARREAR' as const }
    // A flat amount on an extra recurring product stays extra recurring.
    expect(compileCharge(
      { ...preset('platform'), billing: 'arrears' },
      { rate: '1.00' },
      erLock
    ).feeType).toBe('extraRecurring')
    expect(bestFit({ ...preset('platform'), billing: 'arrears' }, { model: 'tiered' }, erLock)).toEqual({
      source: 'fixed',
      model: 'tiered',
      recurrence: 'recurring',
      billing: 'arrears',
    })
    expect(shapeFits({ ...preset('platform'), billing: 'advance' }, erLock)).toBe(false)

    // A usage product prices its metric with any usage model: a per-unit one can go tiered.
    const usageLock = { feeType: 'usage' as const }
    expect(bestFit(preset('platform'), { source: 'metered' }, usageLock)?.model).toBe('per_unit')
    for (const model of ['tiered', 'volume', 'package', 'by_dimension'] as const) {
      expect(bestFit(preset('usage'), { model }, usageLock)?.model).toBe(model)
    }
    expect(shapeFits({ ...preset('usage'), model: 'included' }, usageLock)).toBe(false)
  })

  it('presets are recognized from their shape', () => {
    expect(presetOf(preset('setup'))).toBe('setup')
    expect(presetOf({ ...preset('usage'), model: 'package' })).toBeUndefined()
  })
})

describe('decompileCharge', () => {
  it('reads capacity levels smallest first', () => {
    const { values } = decompileCharge('capacity', {
      thresholds: [
        { included: 250_000, rate: '1499', overageRate: '0.005' },
        { included: 50_000, rate: '479', overageRate: '0.005' },
      ],
    })
    expect((values.thresholds as { included: number }[]).map(level => level.included)).toEqual([
      50_000, 250_000,
    ])
  })

  it('reads stored fee types back into the same shape', () => {
    const cases: [ChargeShape, Record<string, unknown>][] = [
      [preset('platform'), { rate: '10.00' }],
      [preset('setup'), { rate: '500.00' }],
      [preset('seat'), { unitPrice: '12.00' }],
      [{ ...preset('usage'), model: 'volume' }, { metricId: 'm', rows }],
      [preset('usage_included'), { metricId: 'm', thresholds: [] }],
      [{ ...preset('platform'), model: 'package' }, { quantity: 4, packagePrice: '9', blockSize: 2 }],
    ]
    for (const [shape, values] of cases) {
      const compiled = compileCharge(shape, values)
      expect(decompileCharge(compiled.feeType, compiled.formData).shape).toEqual(normalizeShape(shape))
    }
  })
})

describe('valuesForShape', () => {
  it('keeps what was entered and fills the new fields', () => {
    const values = valuesForShape(preset('seat'), { term: 'ANNUAL', rate: '10.00' })
    expect(values).toMatchObject({ term: 'ANNUAL', rate: '10.00', unitPrice: '0.00', slotUnitName: 'Seats' })
  })

  it('by dimension starts with rates in this plan, the tiers kept apart', () => {
    const tiered = { ...preset('usage'), model: 'tiered' as const }
    const byDimension = { ...preset('usage'), model: 'by_dimension' as const }
    const values = valuesForShape(byDimension, { rows }, tiered)
    expect(values).toMatchObject({ rateSource: 'plan', keyRates: [], unitSize: '1' })
    expect(values.rows).toBe(rows)
    expect(valuesForShape({ ...tiered, model: 'volume' }, { rows }, tiered).rows).toBe(rows)
  })
})

describe('pricesKeptOnEdit', () => {
  const prices = [
    { id: 'eur-m', currency: 'EUR', cadence: 0 },
    { id: 'eur-y', currency: 'EUR', cadence: 3 },
    { id: 'usd-m', currency: 'USD', cadence: 0 },
  ]
  const edited = [prices[0]]

  it('keeps the other cadences while the model kind is unchanged', () => {
    expect(pricesKeptOnEdit(prices, edited, 'EUR', 0, true)).toEqual(['eur-y', 'usd-m'])
  })
  it('replaces the price on the cadence moved onto', () => {
    expect(pricesKeptOnEdit(prices, edited, 'EUR', 3, true)).toEqual(['usd-m'])
  })
  it('replaces the currency when the model kind changes', () => {
    expect(pricesKeptOnEdit(prices, edited, 'EUR', 0, false)).toEqual(['usd-m'])
  })
})

describe('summarizeCharge', () => {
  it('reads a seat charge back in one line', () => {
    const line = summarizeCharge(
      preset('seat'),
      { unitPrice: '12', minSlots: 3, slotUnitName: 'Seats' },
      { currency: 'EUR' }
    )
    expect(line).toContain('per seat per month, billed in advance; at least 3 seats')
  })
  it('a setup fee is billed once', () => {
    expect(summarizeCharge(preset('setup'), { rate: '500' }, { currency: 'USD' })).toContain(
      'once, on the first invoice'
    )
  })
})
