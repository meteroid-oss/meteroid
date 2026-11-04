import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  PlanChangePackLine_Kind,
  PlanChangePackLineSchema,
} from '@/rpc/api/subscriptions/v1/models_pb'

import { packLineView } from './planChangePackLines'

const line = (kind: PlanChangePackLine_Kind, amountCents: bigint) =>
  create(PlanChangePackLineSchema, { name: 'Pack', kind, amountCents, creditAmount: '100' })

describe('packLineView', () => {
  it('shows an included pack as the credit it grants, not +$0.00', () => {
    const view = packLineView(line(PlanChangePackLine_Kind.CHARGE, 0n), 'USD', '100 credits')
    expect(view).toEqual({ label: 'Included credit granted', detail: undefined, value: '+100 credits' })
  })

  it('signs a billed pack with + and a fee credit with the minus sign', () => {
    const charge = packLineView(line(PlanChangePackLine_Kind.CHARGE, 500n), 'USD', '100 credits')
    expect(charge?.value).toBe('+$5.00')
    expect(charge?.detail).toBe('100 credits granted')

    const feeCredit = packLineView(line(PlanChangePackLine_Kind.FEE_CREDIT, -250n), 'USD', '')
    expect(feeCredit?.value).toBe('−$2.50')

    const removed = packLineView(line(PlanChangePackLine_Kind.CREDIT_REMOVED, 0n), 'USD', '40 credits')
    expect(removed?.value).toBe('−40 credits')
  })
})
