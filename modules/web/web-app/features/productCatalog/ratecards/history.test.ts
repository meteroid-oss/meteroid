import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { RateCardEntrySchema } from '@/rpc/api/ratecards/v1/models_pb'

import { filterHistory, keyLabel, rateHistory, withdrawal } from './history'

const entry = (key: string[], currency: string, from: string, rate?: string) =>
  create(RateCardEntrySchema, {
    id: `${key.join(',')}-${currency}-${from}`,
    key,
    currency,
    effectiveFrom: from,
    rate,
    anyOther: key.every(p => p === '*'),
  })

describe('rate card history', () => {
  const entries = [
    entry(['*'], 'EUR', '2026-01-01', '1'),
    entry(['x'], 'EUR', '2026-03-01', '4'),
    entry(['x'], 'EUR', '2026-01-01', '3'),
    entry(['x'], 'EUR', '2026-12-01'),
    entry(['x'], 'USD', '2026-01-01', '3.5'),
  ]

  it('lays out each key and currency over time, the rate in force marked', () => {
    const histories = rateHistory(entries, '2026-11-24')
    expect(histories.map(h => `${keyLabel(h.key)} ${h.currency}`)).toEqual([
      'x EUR',
      'x USD',
      'Anything else EUR',
    ])
    expect(histories[0].steps.map(s => [s.from, s.rate, s.standing])).toEqual([
      ['2026-01-01', '3', 'past'],
      ['2026-03-01', '4', 'current'],
      ['2026-12-01', undefined, 'scheduled'],
    ])
  })

  it('finds keys by their text', () => {
    const histories = rateHistory(entries, '2026-11-24')
    expect(filterHistory(histories, 'ANY').map(h => h.anyOther)).toEqual([true])
    expect(filterHistory(histories, ' ')).toHaveLength(3)
  })

  it('tells a scheduled entry replaced on its day from one withdrawn', () => {
    const scheduled = { key: ['x'], currency: 'EUR', effectiveFrom: '2026-12-01' }
    const rates = [{ key: ['x'], currency: 'EUR' }]
    expect(withdrawal(rates, '2026-12-01', scheduled)).toBe('replaced')
    expect(withdrawal(rates, '2026-11-25', scheduled)).toBe('withdrawn')
    expect(withdrawal([], '2026-12-01', scheduled)).toBe('withdrawn')
  })
})
