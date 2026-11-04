import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { WalletDailyUsageCostSchema } from '@/rpc/api/wallets/v1/models_pb'
import { UsageCostPeriod } from '@/rpc/api/wallets/v1/wallets_pb'

import {
  OTHER_SERIES,
  dailyUsageTotal,
  daysBetween,
  seriesColorOrder,
  walletDailyUsageInput,
  walletUsageChartData,
} from './usageChartData'

const row = (date: string, metricName: string, amount: bigint) =>
  create(WalletDailyUsageCostSchema, {
    date,
    metricName,
    metricId: `m_${metricName}`,
    subscriptionId: 'sub_1',
    amount,
    quantity: '1',
  })

describe('walletDailyUsageInput', () => {
  it('asks the period of every subscription, or of the one in scope', () => {
    const wallet = { id: 'w_1', customerId: 'c_1' }
    expect(walletDailyUsageInput(wallet, undefined, 'current')).toEqual({
      customerId: 'c_1',
      walletId: 'w_1',
      subscriptionId: undefined,
      period: UsageCostPeriod.CURRENT,
    })
    expect(walletDailyUsageInput(wallet, 'sub_1', 'last')).toEqual({
      customerId: 'c_1',
      walletId: 'w_1',
      subscriptionId: 'sub_1',
      period: UsageCostPeriod.PREVIOUS,
    })
  })
})

describe('daysBetween', () => {
  it('lists the days up to the exclusive end', () => {
    expect(daysBetween('2026-10-30', '2026-11-02')).toEqual([
      '2026-10-30',
      '2026-10-31',
      '2026-11-01',
    ])
  })
})

describe('walletUsageChartData', () => {
  it('stacks each metric by the day the server attributes it and ranks the consumers', () => {
    const rows = [
      row('2026-10-01', 'Tokens', 100n),
      row('2026-10-02', 'Tokens', 200n),
      row('2026-10-02', 'Images', 100n),
    ]
    const data = walletUsageChartData(rows, { start: '2026-10-01', end: '2026-10-04' })
    expect(data.total).toBe(400n)
    expect(data.total).toBe(dailyUsageTotal(rows))
    expect(data.days.map(d => d.date)).toEqual(['2026-10-01', '2026-10-02', '2026-10-03'])
    expect(data.days[1].values).toEqual({ Tokens: 200n, Images: 100n })
    expect(data.days[2].values).toEqual({})
    expect(data.series).toEqual([
      { key: 'Tokens', total: 300n, share: 75 },
      { key: 'Images', total: 100n, share: 25 },
    ])
  })

  it('adds the days of a volume tier lowering the period price, so the total stays the invoice’s', () => {
    const rows = [
      row('2026-10-01', 'Calls', 6000n),
      row('2026-10-02', 'Calls', -500n),
      row('2026-10-03', 'Calls', 3500n),
    ]
    const data = walletUsageChartData(rows, { start: '2026-10-01', end: '2026-10-04' })
    expect(data.total).toBe(9000n)
    expect(data.days[1].values).toEqual({ Calls: -500n })
    expect(data.series).toEqual([{ key: 'Calls', total: 9000n, share: 100 }])
  })

  it('folds the smallest consumers into Other', () => {
    const data = walletUsageChartData(
      [
        row('2026-10-01', 'A', 50n),
        row('2026-10-01', 'B', 30n),
        row('2026-10-01', 'C', 15n),
        row('2026-10-02', 'D', 5n),
      ],
      { start: '2026-10-01', end: '2026-10-03' },
      3
    )
    expect(data.series.map(s => [s.key, s.total])).toEqual([
      ['A', 50n],
      ['B', 30n],
      [OTHER_SERIES, 20n],
    ])
    expect(data.days[0].values).toEqual({ A: 50n, B: 30n, [OTHER_SERIES]: 15n })
    expect(data.days[1].values).toEqual({ [OTHER_SERIES]: 5n })
  })

  it('keeps days outside the range and needs no range', () => {
    const data = walletUsageChartData([row('2026-09-30', 'A', 10n)], undefined)
    expect(data.days).toEqual([{ date: '2026-09-30', values: { A: 10n } }])
    expect(walletUsageChartData([], undefined)).toEqual({ days: [], series: [], total: 0n })
  })
})

describe('seriesColorOrder', () => {
  it('orders by name, not rank, and leaves Other to its own color', () => {
    expect(seriesColorOrder([{ key: 'Tokens' }, { key: 'Images' }, { key: OTHER_SERIES }])).toEqual(
      ['Images', 'Tokens']
    )
  })
})
