import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import { TrialConfig_Status } from '@/rpc/api/subscriptions/v1/models_pb'
import {
  PortalAfterTrialAmountSchema,
  PortalAfterTrialSchema,
  PortalTrialInvoiceSchema,
  PortalTrialSchema,
} from '@/rpc/portal/subscription/v1/subscription_pb'

import {
  afterTrialAmount,
  payInvoiceLabel,
  paymentMethodLabel,
  trialBadge,
  trialEndedOn,
} from './trial'

describe('trialBadge', () => {
  it('states the end of a running trial', () => {
    const trial = create(PortalTrialSchema, {
      endsOn: '2026-12-15',
      status: TrialConfig_Status.ACTIVE,
    })
    expect(trialBadge(trial)).toEqual({ label: 'Trial · ends Dec 15, 2026', tone: 'warn' })
  })

  it('awaits payment', () => {
    const trial = create(PortalTrialSchema, {
      endsOn: '2026-12-15',
      status: TrialConfig_Status.AWAITING_PAYMENT,
    })
    expect(trialBadge(trial).label).toBe('Awaiting payment')
  })

  it('is still a trial while awaiting a decision', () => {
    const trial = create(PortalTrialSchema, {
      endsOn: '2026-12-15',
      status: TrialConfig_Status.AWAITING_DECISION,
    })
    expect(trialBadge(trial).label).toBe('Trial')
  })
})

describe('afterTrialAmount', () => {
  const after = (...recurring: [bigint, BillingPeriod][]) =>
    create(PortalAfterTrialSchema, {
      currency: 'EUR',
      startsOn: '2026-12-15',
      recurring: recurring.map(([amount, interval]) =>
        create(PortalAfterTrialAmountSchema, { amount, interval })
      ),
    })

  it('is the recurring amount per period', () => {
    expect(afterTrialAmount(after([4900n, BillingPeriod.MONTHLY]), false)).toBe('€49.00/month')
    expect(afterTrialAmount(after([49000n, BillingPeriod.ANNUAL]), false)).toBe('€490.00/year')
  })

  it('states each cadence its fees bill on', () => {
    expect(
      afterTrialAmount(after([49000n, BillingPeriod.ANNUAL], [4000n, BillingPeriod.MONTHLY]), false)
    ).toBe('€490.00/year + €40.00/month')
  })

  it('starts from the recurring amount when usage bills on top', () => {
    expect(afterTrialAmount(after([4900n, BillingPeriod.MONTHLY]), true)).toBe('From €49.00/month')
  })

  it('is usage-based without a recurring fee, and nothing for a plan billing nothing', () => {
    expect(afterTrialAmount(after(), true)).toBe('Usage-based')
    expect(afterTrialAmount(after(), false)).toBeUndefined()
    expect(afterTrialAmount(undefined, true)).toBeUndefined()
  })
})

describe('paymentMethodLabel', () => {
  it('names the method on file, else reads on file', () => {
    const trial = (paymentMethod?: string) =>
      create(PortalTrialSchema, {
        endsOn: '2026-12-15',
        status: TrialConfig_Status.ACTIVE,
        hasPaymentMethod: true,
        paymentMethod,
      })
    expect(paymentMethodLabel(trial('Visa •••• 4242'))).toBe('Visa •••• 4242')
    expect(paymentMethodLabel(trial())).toBe('On file')
  })
})

describe('trialEndedOn', () => {
  it('is when it ended as recorded, else its end', () => {
    const trial = (endedOn?: string) =>
      create(PortalTrialSchema, {
        endsOn: '2026-12-15',
        status: TrialConfig_Status.AWAITING_PAYMENT,
        endedOn,
      })
    expect(trialEndedOn(trial('2026-12-16'))).toBe('2026-12-16')
    expect(trialEndedOn(trial())).toBe('2026-12-15')
  })
})

describe('payInvoiceLabel', () => {
  it('names the amount due', () => {
    const invoice = create(PortalTrialInvoiceSchema, {
      invoiceId: 'inv_1',
      invoiceNumber: 'INV-0142',
      amountDue: 4900n,
      currency: 'EUR',
    })
    expect(payInvoiceLabel(invoice)).toBe('Pay €49.00')
  })
})
