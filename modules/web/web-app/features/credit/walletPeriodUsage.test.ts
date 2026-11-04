import { create } from '@bufbuild/protobuf'
import { timestampFromDate } from '@bufbuild/protobuf/wkt'
import { describe, expect, it } from 'vitest'

import {
  SubscriptionFeeBillingPeriod,
  SubscriptionFeeSchema,
} from '@/rpc/api/subscriptions/v1/models_pb'
import { CreditGrantKind } from '@/rpc/api/wallets/v1/models_pb'

import {
  periodCreditGranted,
  periodUtilization,
  walletAverageUsage,
  walletMonthlyIncluded,
} from './walletPeriodUsage'

describe('walletAverageUsage', () => {
  it('sums each subscription average, counting the most periods averaged', () => {
    expect(walletAverageUsage([[100n, 80n, 90n], [30n], []])).toEqual({ average: 120n, periods: 3 })
  })

  it('is undefined before any period is invoiced', () => {
    expect(walletAverageUsage([[], []])).toBeUndefined()
  })
})

describe('walletMonthlyIncluded', () => {
  const pack = (credit: string, period = SubscriptionFeeBillingPeriod.MONTHLY, once = false) => ({
    period,
    fee: create(SubscriptionFeeSchema, {
      fee: {
        case: 'credit',
        value: {
          rate: '1',
          quantity: 1,
          total: '1',
          creditAmount: credit,
          creditTotal: credit,
          once,
        },
      },
    }),
  })

  it("adds every subscription's monthly pack credit", () => {
    expect(
      walletMonthlyIncluded(
        [
          { currency: 'USD', fees: [pack('550')] },
          { currency: 'USD', fees: [pack('50'), pack('20', undefined, true)] },
        ],
        'USD'
      )?.toString()
    ).toBe('600')
  })

  it('is undefined when a pack is not monthly, never a partial sum', () => {
    expect(
      walletMonthlyIncluded(
        [
          { currency: 'USD', fees: [pack('550')] },
          { currency: 'USD', fees: [pack('600', SubscriptionFeeBillingPeriod.YEARLY)] },
        ],
        'USD'
      )
    ).toBeUndefined()
  })
})

describe('periodCreditGranted and periodUtilization', () => {
  const at = (iso: string) => timestampFromDate(new Date(iso))
  const grants = [
    { kind: CreditGrantKind.INCLUDED, amount: 1000n, effectiveAt: at('2026-10-01T00:00:00Z') },
    { kind: CreditGrantKind.PREPAID, amount: 100n, effectiveAt: at('2026-10-10T00:00:00Z') },
    {
      kind: CreditGrantKind.BONUS,
      amount: 50n,
      effectiveAt: at('2026-10-12T00:00:00Z'),
      subscriptionId: 'sub_2',
    },
    { kind: CreditGrantKind.PREPAID, amount: 999n, effectiveAt: at('2026-09-10T00:00:00Z') },
    { kind: CreditGrantKind.ACCOUNT_BALANCE, amount: 70n, effectiveAt: at('2026-10-05T00:00:00Z') },
    { kind: CreditGrantKind.INCLUDED, amount: 1000n, effectiveAt: at('2026-11-01T00:00:00Z') },
  ]
  const now = new Date('2026-10-22T00:00:00Z')

  it('splits the credit that took effect this period into the plan and what was added', () => {
    expect(periodCreditGranted(grants, '2026-10-01', now)).toEqual({ plan: 1000n, added: 150n })
    expect(periodCreditGranted(grants, '2026-10-01', now, 'sub_1')).toEqual({
      plan: 1000n,
      added: 100n,
    })
  })

  it('states the share used of it, only with plan credit', () => {
    expect(periodUtilization(870n, { plan: 1000n, added: 150n })).toEqual({
      total: 1150n,
      label: '76%',
    })
    expect(periodUtilization(1n, { plan: 1000n, added: 0n })?.label).toBe('<1%')
    expect(periodUtilization(500n, { plan: 0n, added: 600n })).toBeUndefined()
  })
})
