import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { CreditUnitSchema } from '@/rpc/api/creditunits/v1/models_pb'
import { UsageFeeSchema } from '@/rpc/api/pricecomponents/v1/models_pb'
import {
  SubscriptionFee_CapacitySubscriptionFeeSchema,
  SubscriptionFee_CreditSubscriptionFeeSchema,
  SubscriptionFee_RateSubscriptionFeeSchema,
  SubscriptionFeeSchema,
  SubscriptionFeeBillingPeriod,
} from '@/rpc/api/subscriptions/v1/models_pb'

import { canOverridePrice, formatAmendFee, subscriptionFeeToFormData } from './StepEditChanges'

// fix G: overriding a usage/capacity component on amend must keep the live credit-unit pricing
// (creditUnitId/creditUnitRate) so the override form doesn't silently drop it back to money.
describe('subscriptionFeeToFormData (fix G: keeps credit-unit pricing on amend overrides)', () => {
  it('keeps creditUnitId/creditUnitRate for a usage fee priced in a credit unit', () => {
    const fee = create(SubscriptionFeeSchema, {
      creditUnit: { creditUnitId: 'cu_42', code: 'CREDITS', precision: 0, rate: '0.001' },
    })
    fee.fee = {
      case: 'usage',
      value: create(UsageFeeSchema, { metricId: 'metric_1', model: { case: 'perUnit', value: '2' } }),
    }

    const formData = subscriptionFeeToFormData(fee, SubscriptionFeeBillingPeriod.MONTHLY)

    expect(formData).toMatchObject({
      metricId: 'metric_1',
      usageModel: 'per_unit',
      unitPrice: '2',
      creditUnitId: 'cu_42',
      creditUnitRate: '0.001',
    })
  })

  it('keeps creditUnitId/creditUnitRate for a capacity fee priced in a credit unit', () => {
    const fee = create(SubscriptionFeeSchema, {
      creditUnit: { creditUnitId: 'cu_7', code: 'CREDITS', precision: 0, rate: '0.002' },
    })
    fee.fee = {
      case: 'capacity',
      value: create(SubscriptionFee_CapacitySubscriptionFeeSchema, {
        rate: '10',
        included: 1000n,
        overageRate: '0.5',
        metricId: 'metric_2',
      }),
    }

    const formData = subscriptionFeeToFormData(fee, SubscriptionFeeBillingPeriod.MONTHLY)

    expect(formData).toMatchObject({
      rate: '10',
      included: 1000,
      overageRate: '0.5',
      creditUnitId: 'cu_7',
      creditUnitRate: '0.002',
    })
  })

  it('defaults creditUnitId/creditUnitRate to empty string when the fee is priced in money', () => {
    const fee = create(SubscriptionFeeSchema)
    fee.fee = {
      case: 'usage',
      value: create(UsageFeeSchema, { metricId: 'metric_3', model: { case: 'perUnit', value: '5' } }),
    }

    const formData = subscriptionFeeToFormData(fee, SubscriptionFeeBillingPeriod.MONTHLY)

    expect(formData).toMatchObject({ creditUnitId: '', creditUnitRate: '' })
  })

  it('prefills the current matrix and tiered prices', () => {
    const matrix = create(SubscriptionFeeSchema)
    matrix.fee = {
      case: 'usage',
      value: create(UsageFeeSchema, {
        metricId: 'tokens',
        model: {
          case: 'matrix',
          value: {
            rows: [
              { perUnitPrice: '2', dimension1: { key: 'model', value: 'halyard-lite' } },
              { perUnitPrice: '60', dimension1: { key: 'model', value: 'halyard-max' } },
            ],
          },
        },
      }),
    }
    expect(subscriptionFeeToFormData(matrix, SubscriptionFeeBillingPeriod.MONTHLY)).toMatchObject({
      usageModel: 'matrix',
      rows: [
        { perUnitPrice: '2', dimension1: { key: 'model', value: 'halyard-lite' } },
        { perUnitPrice: '60', dimension1: { key: 'model', value: 'halyard-max' } },
      ],
    })

    const tiered = create(SubscriptionFeeSchema)
    tiered.fee = {
      case: 'usage',
      value: create(UsageFeeSchema, {
        metricId: 'tokens',
        model: {
          case: 'tiered',
          value: {
            rows: [
              { firstUnit: 0n, unitPrice: '1' },
              { firstUnit: 100n, unitPrice: '0.5' },
            ],
          },
        },
      }),
    }
    expect(subscriptionFeeToFormData(tiered, SubscriptionFeeBillingPeriod.MONTHLY)).toMatchObject({
      usageModel: 'tiered',
      rows: [
        { firstUnit: 0n, unitPrice: '1' },
        { firstUnit: 100n, unitPrice: '0.5' },
      ],
    })
  })

  it('returns undefined when there is no fee', () => {
    expect(subscriptionFeeToFormData(undefined, SubscriptionFeeBillingPeriod.MONTHLY)).toBeUndefined()
  })
})

// RF-30: a pack's fee does not carry its expiry, so its price is never overridden in place.
describe('canOverridePrice', () => {
  it('refuses a credit pack', () => {
    const fee = create(SubscriptionFeeSchema, {
      creditUnit: { creditUnitId: 'cu_1', code: 'API', precision: 0 },
    })
    fee.fee = {
      case: 'credit',
      value: create(SubscriptionFee_CreditSubscriptionFeeSchema, {
        rate: '50',
        quantity: 1,
        total: '50',
        creditAmount: '5000',
        creditTotal: '5000',
      }),
    }
    expect(canOverridePrice(fee)).toBe(false)
  })

  it('allows other fees', () => {
    const fee = create(SubscriptionFeeSchema)
    fee.fee = { case: 'rate', value: create(SubscriptionFee_RateSubscriptionFeeSchema, { rate: '10' }) }
    expect(canOverridePrice(fee)).toBe(true)
  })
})

describe('formatAmendFee (RF-31)', () => {
  it('shows the credit a unit pack grants in its unit, not as money', () => {
    const fee = create(SubscriptionFeeSchema, {
      creditUnit: { creditUnitId: 'cu_1', code: 'API', precision: 0 },
    })
    fee.fee = {
      case: 'credit',
      value: create(SubscriptionFee_CreditSubscriptionFeeSchema, {
        rate: '50',
        quantity: 1,
        total: '50',
        creditAmount: '5000',
        creditTotal: '5000',
      }),
    }
    const line = formatAmendFee(fee, 'USD', [
      create(CreditUnitSchema, { id: 'cu_1', name: 'API credit', pluralName: 'API credits' }),
    ])
    expect(line).toContain('$50')
    expect(line).toContain('5,000 API credits')
    expect(line).not.toContain('$5,000')
  })
})
