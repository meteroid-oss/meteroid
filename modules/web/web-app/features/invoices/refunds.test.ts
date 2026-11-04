import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { creditNoteRefundLabel, reversedLabel } from '@/features/invoices/refunds'
import { CreditNoteStatus } from '@/rpc/api/creditnotes/v1/models_pb'
import { TransactionSchema, Transaction_ReversalKind } from '@/rpc/api/invoices/v1/models_pb'

describe('reversedLabel', () => {
  it('says a disputed payment was charged back, not refunded', () => {
    expect(
      reversedLabel(
        create(TransactionSchema, { reversalKind: Transaction_ReversalKind.REVERSAL_CHARGEBACK })
      )
    ).toBe('Charged back')
    expect(reversedLabel(create(TransactionSchema, { amountDisputed: 2_500n }))).toBe(
      'Charged back'
    )
    expect(
      reversedLabel(
        create(TransactionSchema, { reversalKind: Transaction_ReversalKind.REVERSAL_RETURNED })
      )
    ).toBe('Reversed')
    expect(
      reversedLabel(
        create(TransactionSchema, { reversalKind: Transaction_ReversalKind.REVERSAL_REFUND })
      )
    ).toBe('Refunded')
    expect(reversedLabel(create(TransactionSchema, {}))).toBe('Refunded')
  })
})

describe('creditNoteRefundLabel', () => {
  const note = (settled: bigint, pending: bigint, status = CreditNoteStatus.FINALIZED) => ({
    status,
    refundedAmountCents: 3900n,
    refundedSettledCents: settled,
    refundedPendingCents: pending,
  })

  it('says what was paid out of the refund the note records', () => {
    expect(creditNoteRefundLabel(note(0n, 0n))).toBe('To refund')
    expect(creditNoteRefundLabel(note(0n, 3900n))).toBe('Refund in progress')
    expect(creditNoteRefundLabel(note(1000n, 2900n))).toBe('Refund in progress')
    expect(creditNoteRefundLabel(note(1000n, 0n))).toBe('Partly refunded')
    expect(creditNoteRefundLabel(note(3900n, 0n))).toBe('Refunded')
  })

  it('never says To refund on a voided note', () => {
    expect(creditNoteRefundLabel(note(0n, 0n, CreditNoteStatus.VOIDED))).toBeUndefined()
    expect(creditNoteRefundLabel(note(1000n, 0n, CreditNoteStatus.VOIDED))).toBe('Refunded')
  })
})
