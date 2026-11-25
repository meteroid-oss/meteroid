import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  FeeStructureSchema,
  FeeStructure_BillingType,
  FeeStructure_UsageModel,
} from '@/rpc/api/prices/v1/models_pb'

import { chargesForWords, colorLabel, iconLabel } from './productWords'

const structure = (s: object) => create(FeeStructureSchema, { structure: s as never })

describe('chargesForWords', () => {
  it('says what a seat product counts', () => {
    expect(chargesForWords(structure({ case: 'slot', value: { unitName: 'seat' } }))).toEqual([
      'Seats',
      'counts seat',
    ])
    expect(chargesForWords(structure({ case: 'slot', value: { unitName: 'Editors' } }))).toEqual([
      'Seats',
      'counts editor',
    ])
  })

  it('names the metric usage reads, and its model', () => {
    const usage = structure({
      case: 'usage',
      value: { metricId: 'm1', model: FeeStructure_UsageModel.TIERED },
    })
    expect(chargesForWords(usage, 'Tokens')).toEqual([
      'Usage',
      { metricId: 'm1', name: 'Tokens' },
      'tiered',
    ])
    expect(chargesForWords(usage)[1]).toEqual({ metricId: 'm1', name: 'its metric' })
  })

  it('says when a recurring fee is billed', () => {
    const fee = (billingType: FeeStructure_BillingType) =>
      chargesForWords(structure({ case: 'extraRecurring', value: { billingType } }))
    expect(fee(FeeStructure_BillingType.ARREAR)).toEqual(['Recurring fee', 'billed in arrears'])
    expect(fee(FeeStructure_BillingType.ADVANCE)).toEqual(['Recurring fee', 'billed in advance'])
  })

  it('reads a flat fee, and nothing without a structure', () => {
    expect(chargesForWords(structure({ case: 'rate', value: {} }))).toEqual(['Flat fee'])
    expect(chargesForWords(undefined)).toEqual([])
  })
})

describe('appearance labels', () => {
  it('reads icon and color tokens as words', () => {
    expect(iconLabel('message-square')).toBe('Message square')
    expect(iconLabel(undefined)).toBe('Box')
    expect(colorLabel('blue')).toBe('Blue')
    expect(colorLabel(undefined)).toBe('Neutral')
  })
})
