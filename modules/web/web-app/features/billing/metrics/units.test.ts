import { describe, expect, it } from 'vitest'

import {
  formatMetricQuantity,
  formatPerUnit,
  formatPricePerUnit,
  formatRawQuantity,
  metricUnitFromProto,
  rawQuantity,
  unitNoun,
} from './units'

const tokens = { singular: 'token', plural: 'tokens' }

describe('formatPerUnit', () => {
  it('uses the singular when there is no conversion factor', () => {
    expect(formatPerUnit(tokens)).toBe('per token')
    expect(formatPerUnit({ ...tokens, conversionFactor: 1 })).toBe('per token')
  })

  it('abbreviates exact 1K / 1M / 1B factors with the plural', () => {
    expect(formatPerUnit({ ...tokens, conversionFactor: 1_000 })).toBe('per 1K tokens')
    expect(formatPerUnit({ ...tokens, conversionFactor: 1_000_000 })).toBe('per 1M tokens')
    expect(formatPerUnit({ ...tokens, conversionFactor: 1_000_000_000 })).toBe('per 1B tokens')
  })

  it('groups any other factor', () => {
    expect(formatPerUnit({ ...tokens, conversionFactor: 2_500 })).toBe('per 2,500 tokens')
  })

  it('falls back to unit/units without a label', () => {
    expect(formatPerUnit(undefined)).toBe('per unit')
    expect(formatPerUnit({ conversionFactor: 1_000 })).toBe('per 1K units')
    expect(formatPerUnit({ conversionFactor: 1_000_000 })).toBe('per 1M units')
  })
})

describe('formatPricePerUnit', () => {
  it('appends the per-unit suffix to a pre-formatted price', () => {
    expect(formatPricePerUnit('$12.00', { ...tokens, conversionFactor: 1_000_000 })).toBe(
      '$12.00 per 1M tokens'
    )
  })
})

describe('formatMetricQuantity', () => {
  it('formats compact quantities', () => {
    expect(formatMetricQuantity(9_004_601, tokens, { compact: true })).toBe('9.0M tokens')
    expect(formatMetricQuantity(1, tokens, { compact: true })).toBe('1 token')
    expect(formatMetricQuantity(950, tokens, { compact: true })).toBe('950 tokens')
    expect(formatMetricQuantity(1_234, tokens, { compact: true })).toBe('1.2K tokens')
    expect(formatMetricQuantity(0.5, tokens, { compact: true })).toBe('0.5 tokens')
  })

  it('formats full quantities with grouping', () => {
    expect(formatMetricQuantity(9_004_601, tokens)).toBe('9,004,601 tokens')
    expect(formatMetricQuantity('9004601', tokens)).toBe('9,004,601 tokens')
  })

  it('falls back to unit/units without a label', () => {
    expect(formatMetricQuantity(1, undefined)).toBe('1 unit')
    expect(formatMetricQuantity(2, undefined)).toBe('2 units')
  })

  it('renders non-numeric strings as-is', () => {
    expect(formatMetricQuantity('n/a', tokens)).toBe('n/a tokens')
  })
})

describe('unitNoun', () => {
  it('picks singular only for |quantity| === 1', () => {
    expect(unitNoun(1, tokens)).toBe('token')
    expect(unitNoun(-1, tokens)).toBe('token')
    expect(unitNoun(0, tokens)).toBe('tokens')
    expect(unitNoun(2, undefined)).toBe('units')
  })
})

describe('metricUnitFromProto', () => {
  it('reads the factor from the aggregation or the flat field', () => {
    expect(
      metricUnitFromProto({ unitLabel: tokens, aggregation: { unitConversion: { factor: 1e6 } } })
    ).toEqual({ ...tokens, conversionFactor: 1e6 })
    expect(metricUnitFromProto({ unitConversionFactor: 1_000 })).toEqual({
      singular: null,
      plural: null,
      conversionFactor: 1_000,
    })
    expect(metricUnitFromProto(undefined)).toBeUndefined()
  })
})

describe('raw quantities', () => {
  const perMillion = { ...tokens, conversionFactor: 1_000_000 }

  it('scales a priced quantity back to raw units', () => {
    expect(rawQuantity('1.5', perMillion)).toBe(1_500_000)
    expect(rawQuantity(3n, undefined)).toBe(3)
    expect(rawQuantity('n/a', perMillion)).toBeNull()
  })

  it('formats raw units without the noun', () => {
    expect(formatRawQuantity('1.5', perMillion)).toBe('1,500,000')
    expect(formatRawQuantity(undefined, perMillion)).toBe('—')
    expect(formatRawQuantity('n/a', perMillion)).toBe('n/a')
  })
})
