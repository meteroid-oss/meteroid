import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  SubscriptionFeeBillingPeriod,
  SubscriptionFeeSchema,
} from '@/rpc/api/subscriptions/v1/models_pb'

import { feeBillingPeriod, formatSubscriptionFee } from './fees'

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
