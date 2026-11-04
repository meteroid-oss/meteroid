import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  allowedDispositions,
  creditToBalanceBlocked,
  estimatedNoteTotal,
  exceedsAmountDue,
  fullScopeBlocker,
  quantityError,
  reissueBlocked,
} from '@/features/invoices/creditNoteRules'
import {
  InvoicePaymentStatus,
  LineItemKind,
  LineItemSchema,
  TransactionSchema,
  Transaction_PaymentStatusEnum,
  Transaction_PaymentTypeEnum,
} from '@/rpc/api/invoices/v1/models_pb'

const invoice = (overrides: Partial<Parameters<typeof fullScopeBlocker>[0]> = {}) => ({
  paymentStatus: InvoicePaymentStatus.UNPAID,
  transactions: [],
  appliedCredits: 0n,
  amountDue: 12_000n,
  total: 12_000n,
  ...overrides,
})

const pack = create(LineItemSchema, {
  kind: LineItemKind.CREDIT,
  creditGrantAmount: 50_000n,
  subtotal: 5_000n,
})
const seats = create(LineItemSchema, { kind: LineItemKind.REGULAR, subtotal: 10_000n })
const includedPack = create(LineItemSchema, {
  kind: LineItemKind.CREDIT,
  creditGrantAmount: 50_000n,
  subtotal: 0n,
})

describe('credit-note dialog rules', () => {
  it('blocks a full cancellation the server would refuse', () => {
    expect(fullScopeBlocker(invoice())).toBeNull()
    expect(fullScopeBlocker(invoice({ paymentStatus: InvoicePaymentStatus.PARTIALLY_PAID }))).toBe(
      'partially-paid'
    )
    // Credit paid 30.00 of 120.00: only 90.00 is debt left to cancel.
    expect(fullScopeBlocker(invoice({ appliedCredits: 3_000n, amountDue: 9_000n }))).toBe(
      'credit-paid-part'
    )
    const pending = create(TransactionSchema, {
      paymentType: Transaction_PaymentTypeEnum.PAYMENT,
      status: Transaction_PaymentStatusEnum.PENDING,
    })
    // A payment in flight leaves the whole invoice open to a credit-to-balance note.
    expect(fullScopeBlocker(invoice({ transactions: [pending] }))).toBeNull()
    expect(allowedDispositions(invoice({ transactions: [pending] }), 'full')).toEqual([
      'credit-to-balance',
    ])
    expect(
      allowedDispositions(invoice({ paymentStatus: InvoicePaymentStatus.PROCESSING }), 'partial')
    ).toEqual(['credit-to-balance'])
    expect(
      allowedDispositions(
        invoice({ paymentStatus: InvoicePaymentStatus.PARTIALLY_PAID, transactions: [pending] }),
        'partial'
      )
    ).toEqual(['refund', 'credit-to-balance'])
    expect(fullScopeBlocker(invoice({ paymentStatus: InvoicePaymentStatus.PAID }))).toBeNull()
  })

  it('offers the dispositions each payment state allows', () => {
    expect(allowedDispositions(invoice(), 'full')).toEqual(['reduce-debt'])
    expect(
      allowedDispositions(invoice({ paymentStatus: InvoicePaymentStatus.PAID }), 'full')
    ).toEqual(['refund', 'credit-to-balance'])
  })

  it('never returns sold credit to the balance, and never reissues a capped refund', () => {
    expect(creditToBalanceBlocked([seats])).toBe(false)
    expect(creditToBalanceBlocked([seats, pack])).toBe(true)
    expect(creditToBalanceBlocked([includedPack])).toBe(false)
    expect(reissueBlocked('full', 'refund', [seats, pack])).toBe(true)
    expect(reissueBlocked('full', 'credit-to-balance', [seats])).toBe(false)
    expect(reissueBlocked('partial', 'refund', [pack])).toBe(false)
  })

  it('validates a credited quantity against the line', () => {
    expect(quantityError('', '1', 0, 10_000)).toBeNull()
    expect(quantityError('1', '1', 10_000, 10_000)).toBeNull()
    expect(quantityError('50', '1', 500_000, 10_000)).toEqual({
      kind: 'above-original',
      original: '1',
    })
    expect(quantityError('0', '1', 0, 10_000)).toEqual({ kind: 'not-positive' })
    // Half the line was credited already: the full quantity no longer fits.
    expect(quantityError('1', '1', 10_000, 5_000)).toEqual({ kind: 'above-remaining' })
  })

  it('caps a debt cancellation at the amount due, tax included', () => {
    // A: 100.00 less 50.00 of promotional credit, 10.00 of VAT on the 50.00 left; credit paid
    // 20.00 of the 60.00, so 40.00 is due. A 66.66 line credit is 33.33 + 6.67 = 40.00 and fits;
    // 66.67 prorates to 33.34 + 6.67 = 40.01, which the server refuses too.
    const a = create(LineItemSchema, {
      subtotal: 10_000n,
      taxableAmount: 5_000n,
      taxAmount: 1_000n,
    })
    expect(estimatedNoteTotal(a, 6_666)).toBe(4_000n)
    expect(
      exceedsAmountDue('reduce-debt', estimatedNoteTotal(a, 6_666), { amountDue: 4_000n })
    ).toBe(false)
    expect(
      exceedsAmountDue('reduce-debt', estimatedNoteTotal(a, 6_667), { amountDue: 4_000n })
    ).toBe(true)
    // B: no discount, 20% VAT, 100.00 due: a 100.00 line credit is 120.00 and is refused.
    const b = create(LineItemSchema, {
      subtotal: 10_000n,
      taxableAmount: 10_000n,
      taxAmount: 2_000n,
    })
    expect(estimatedNoteTotal(b, 10_000)).toBe(12_000n)
    expect(
      exceedsAmountDue('reduce-debt', estimatedNoteTotal(b, 10_000), { amountDue: 10_000n })
    ).toBe(true)
    expect(exceedsAmountDue('refund', 12_000n, { amountDue: 0n })).toBe(false)
  })
})
