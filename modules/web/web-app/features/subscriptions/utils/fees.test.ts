import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  SubscriptionFeeBillingPeriod,
  SubscriptionFeeSchema,
} from '@/rpc/api/subscriptions/v1/models_pb'

import { feeBillingPeriod, formatSubscriptionFee, slotFeeTotal } from './fees'

const studioPack = (names?: { name: string; pluralName: string }) =>
  create(SubscriptionFeeSchema, {
    fee: {
      case: 'credit',
      value: { rate: '49', quantity: 1, total: '49', creditAmount: '5000', creditTotal: '5000' },
    },
    creditUnit: { creditUnitId: 'unit-1', code: 'HALYARD', precision: 0, ...names },
  })

describe('formatSubscriptionFee', () => {
  it('names credits from the names the fee carries when the units are not loaded', () => {
    const fee = studioPack({ name: 'Halyard credit', pluralName: 'Halyard credits' })
    expect(formatSubscriptionFee(fee, 'USD').details).toBe('5,000 Halyard credits')
  })

  it("prefers the tenant's units, and stays generic without either", () => {
    const units = [{ id: 'unit-1', name: 'Token', pluralName: 'Tokens' }]
    expect(formatSubscriptionFee(studioPack(), 'USD', units).details).toBe('5,000 Tokens')
    expect(formatSubscriptionFee(studioPack(), 'USD').details).toBe('5,000 credits')
  })

  it('groups the digits of a capacity allowance', () => {
    const fee = create(SubscriptionFeeSchema, {
      fee: { case: 'capacity', value: { rate: '4600', included: 500000n, overageRate: '0' } },
    })
    expect(formatSubscriptionFee(fee, 'EUR').breakdown).toBe('500,000 included')
  })
})

const tiers = (...rows: [bigint, string][]) => ({
  model: {
    case: 'tiered' as const,
    value: { rows: rows.map(([firstUnit, unitPrice]) => ({ firstUnit, unitPrice })) },
  },
})

describe('formatSubscriptionFee reads a priced count', () => {
  // "3 included, then $10": its first rate is $0, never what a seat costs.
  const seats = create(SubscriptionFeeSchema, {
    fee: {
      case: 'slot',
      value: { unit: 'seat', unitRate: '', initialSlots: 5, model: tiers([0n, '0'], [3n, '10']) },
    },
  })

  it('in words, never as "$0.00 per seat"', () => {
    expect(formatSubscriptionFee(seats, 'USD').amount).toBe('First 3 free, then $10 per seat')
  })

  it('totals the seats through the model', () => {
    if (seats.fee.case !== 'slot') throw new Error('a seat fee')
    expect(slotFeeTotal(seats.fee.value, 5).toString()).toBe('20')
    expect(slotFeeTotal({ ...seats.fee.value, model: undefined, unitRate: '12' }, 5).toString()).toBe(
      '60'
    )
  })

  it('a fixed quantity as its count times its price, or its model in words', () => {
    const recurring = (model?: ReturnType<typeof tiers>) =>
      create(SubscriptionFeeSchema, {
        fee: {
          case: 'recurring',
          value: { rate: model ? '0' : '9.99', quantity: 15, total: '140', billingType: 1, model },
        },
      })
    expect(formatSubscriptionFee(recurring(), 'USD').details).toBe('15 × $9.99 (in advance)')
    expect(formatSubscriptionFee(recurring(tiers([0n, '10'], [10n, '8'])), 'USD').details).toBe(
      '15 units: first 10 at $10, then $8 each (in advance)'
    )
  })
})

describe('feeBillingPeriod', () => {
  const welcome = create(SubscriptionFeeSchema, {
    fee: { case: 'credit', value: { rate: '0', creditAmount: '50', once: true } },
  })

  it('bills a credit granted once at start once, whatever its plan cadence', () => {
    const monthly = SubscriptionFeeBillingPeriod.MONTHLY
    expect(feeBillingPeriod(monthly, welcome)).toBe(SubscriptionFeeBillingPeriod.ONE_TIME)
    expect(feeBillingPeriod(monthly, studioPack())).toBe(monthly)
    expect(feeBillingPeriod(monthly, undefined)).toBe(monthly)
  })
})

describe('formatSubscriptionFee reads usage tiers as billing prices them', () => {
  const usage = (kind: 'tiered' | 'volume') =>
    create(SubscriptionFeeSchema, {
      fee: {
        case: 'usage',
        value: {
          metricId: 'm',
          model: {
            case: kind,
            value: {
              rows: [
                { firstUnit: 0n, unitPrice: '1' },
                { firstUnit: 100n, unitPrice: '0.5' },
              ],
            },
          },
        },
      },
    })
  const ranges = (kind: 'tiered' | 'volume') =>
    formatSubscriptionFee(usage(kind), 'USD')
      .breakdown?.split('\n')
      .map(line => line.split(':')[0])

  it('graduated tiers by their own units, the first counted from 1', () => {
    expect(ranges('tiered')).toEqual(['1-100', '101-∞'])
  })

  it('volume tiers by the total they price, the next tier taking its start', () => {
    expect(ranges('volume')).toEqual(['0-99', '100-∞'])
  })
})
