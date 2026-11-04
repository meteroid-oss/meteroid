import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { FeeType } from '@/rpc/api/prices/v1/models_pb'
import { SubscriptionFeeBillingPeriod } from '@/rpc/api/subscriptions/v1/models_pb'
import {
  ComponentFeeSchema,
  type ComponentFee,
} from '@/rpc/portal/subscription/v1/subscription_pb'

import { feeLabel, money, moneyStr, nextPayment, rateStr } from './format'

describe('money', () => {
  it('formats a 2-decimal currency (USD) from cents, unchanged from prior behaviour', () => {
    expect(money(12345, 'USD')).toBe('$123.45')
    expect(money(29000, 'USD')).toBe('$290.00')
  })

  it('formats a 0-decimal currency (JPY) at its own precision, not /100', () => {
    expect(money(12345, 'JPY')).toBe('¥12,345')
  })

  it('formats a 3-decimal currency (KWD) at its own precision', () => {
    expect(money(12345, 'KWD')).toBe('KWD\u00A012.345')
  })

  it('compact trims trailing zeros only for a whole amount', () => {
    expect(money(29000, 'USD', { compact: true })).toBe('$290')
    expect(money(12345, 'USD', { compact: true })).toBe('$123.45')
  })
})

describe('moneyStr', () => {
  it('formats a decimal-string amount for a 2-decimal currency', () => {
    expect(moneyStr('40', 'USD')).toBe('$40.00')
    expect(moneyStr('40.5', 'USD')).toBe('$40.50')
  })

  it('formats a decimal-string amount for JPY (0 decimals) without scaling by 100', () => {
    expect(moneyStr('15000', 'JPY')).toBe('¥15,000')
  })

  it('formats a decimal-string amount for KWD (3 decimals)', () => {
    expect(moneyStr('12.345', 'KWD')).toBe('KWD\u00A012.345')
  })

  it('returns the raw string when it is not a number', () => {
    expect(moneyStr('not-a-number', 'USD')).toBe('not-a-number')
  })
})

describe('rateStr', () => {
  it('keeps sub-cent precision exact instead of rounding to 2 decimals', () => {
    expect(rateStr('0.001', 'USD')).toBe('$0.001')
    expect(rateStr('0.0005', 'USD')).toBe('$0.0005')
  })

  it('pads a whole rate to the usual 2 decimals', () => {
    expect(rateStr('290', 'USD')).toBe('$290.00')
  })
})

describe('feeLabel credit pack', () => {
  const creditFee = (): ComponentFee =>
    create(ComponentFeeSchema, {
      feeType: FeeType.CREDIT,
      amount: '40',
      cadence: SubscriptionFeeBillingPeriod.MONTHLY,
      creditAmount: '5000',
      creditUnit: { creditUnitId: 'cu_1', code: 'HALYARD_CREDITS', precision: 0 },
      creditUnitLabel: {
        code: 'HALYARD_CREDITS',
        name: 'Halyard credit',
        pluralName: 'Halyard credits',
        precision: 0,
      },
    })

  it('names the credit amount by the unit, never its code', () => {
    expect(feeLabel(creditFee(), 'USD')).toBe('5,000 Halyard credits for $40.00 /mo')
  })

  it('says "credits" when the unit label is missing, still never the code', () => {
    const fee = creditFee()
    fee.creditUnitLabel = undefined
    expect(feeLabel(fee, 'USD')).toBe('5,000 credits for $40.00 /mo')
  })

  it('falls back to the money price alone when the pack has no credit amount', () => {
    const fee = create(ComponentFeeSchema, {
      feeType: FeeType.CREDIT,
      amount: '40',
      cadence: SubscriptionFeeBillingPeriod.MONTHLY,
    })
    expect(feeLabel(fee, 'USD')).toBe('$40.00 /mo')
  })

  it('returns an em dash when there is no fee', () => {
    expect(feeLabel(undefined, 'USD')).toBe('—')
  })
})

describe('feeLabel usage priced in a credit unit', () => {
  it('names the unit in the rate note', () => {
    const fee = create(ComponentFeeSchema, {
      feeType: FeeType.USAGE,
      amount: '',
      cadence: SubscriptionFeeBillingPeriod.MONTHLY,
      creditUnit: { creditUnitId: 'cu_1', code: 'HALYARD_CREDITS', precision: 0, rate: '0.01' },
      creditUnitLabel: {
        code: 'HALYARD_CREDITS',
        name: 'Halyard credit',
        pluralName: 'Halyard credits',
        precision: 0,
      },
    })
    expect(feeLabel(fee, 'USD')).toBe('Usage-based, in Halyard credits (1 Halyard credit = $0.01)')
  })
})

describe('nextPayment', () => {
  const invoice = {
    chargeAutomatically: true,
    chargeDate: '2026-11-19',
    dueDate: '2026-11-19',
    invoiceDate: '2026-11-18',
  }

  it('is the charge date when the invoice is charged to a payment method', () => {
    expect(nextPayment({ ...invoice, paymentMethod: 'pm_1' })).toEqual({
      charged: true,
      date: '2026-11-19',
      paymentMethod: 'pm_1',
    })
  })

  it('is the due date, never the invoice date, when nothing is charged automatically', () => {
    expect(nextPayment(invoice)).toEqual({ charged: false, date: '2026-11-19' })
    expect(nextPayment({ ...invoice, chargeAutomatically: false, paymentMethod: 'pm_1' })).toEqual(
      { charged: false, date: '2026-11-19' }
    )
    expect(nextPayment({ ...invoice, dueDate: undefined }).date).toBe('2026-11-18')
  })
})
