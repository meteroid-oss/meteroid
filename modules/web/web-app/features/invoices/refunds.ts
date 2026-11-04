import { CreditNoteStatus } from '@/rpc/api/creditnotes/v1/models_pb'
import {
  Transaction,
  Transaction_PaymentStatusEnum,
  Transaction_PaymentTypeEnum,
  Transaction_RefundMode,
  Transaction_ReversalKind,
} from '@/rpc/api/invoices/v1/models_pb'

export const REVERSAL_KIND_LABELS: Record<Transaction_ReversalKind, string> = {
  [Transaction_ReversalKind.REVERSAL_REFUND]: 'Refund',
  [Transaction_ReversalKind.REVERSAL_CHARGEBACK]: 'Chargeback',
  [Transaction_ReversalKind.REVERSAL_DEBTOR_RECALL]: 'Debtor recall',
  [Transaction_ReversalKind.REVERSAL_INSUFFICIENT_FUNDS]: 'Insufficient funds',
  [Transaction_ReversalKind.REVERSAL_MANDATE_INVALID]: 'Mandate invalid',
  [Transaction_ReversalKind.REVERSAL_RETURNED]: 'Returned',
  [Transaction_ReversalKind.REVERSAL_DISPUTE]: 'Dispute',
  [Transaction_ReversalKind.REVERSAL_OTHER]: 'Other',
}

/**
 * The note's refund share in its totals, as far as it has been paid out: recording a refund
 * note issues no refund, the refunds made from it do.
 */
export const creditNoteRefundLabel = (note: {
  status: CreditNoteStatus
  refundedAmountCents: bigint
  refundedSettledCents: bigint
  refundedPendingCents: bigint
}): 'Refunded' | 'Refund in progress' | 'Partly refunded' | 'To refund' | undefined => {
  if (note.refundedSettledCents >= note.refundedAmountCents) return 'Refunded'
  // A voided note refunds nothing more: only what was already paid out is stated.
  if (note.status === CreditNoteStatus.VOIDED) {
    return note.refundedSettledCents > 0n ? 'Refunded' : undefined
  }
  if (note.refundedPendingCents > 0n) return 'Refund in progress'
  if (note.refundedSettledCents > 0n) return 'Partly refunded'
  return 'To refund'
}

/** The payer's claim: the invoice stays paid and the loss is booked (others reopen it). */
export const DISPUTE_REVERSAL_KINDS: Transaction_ReversalKind[] = [
  Transaction_ReversalKind.REVERSAL_CHARGEBACK,
  Transaction_ReversalKind.REVERSAL_DEBTOR_RECALL,
  Transaction_ReversalKind.REVERSAL_DISPUTE,
]

/** Offered when clawing back a manual payment, worst-to-mildest cause. */
export const REVERSAL_KINDS: Transaction_ReversalKind[] = [
  Transaction_ReversalKind.REVERSAL_CHARGEBACK,
  Transaction_ReversalKind.REVERSAL_DEBTOR_RECALL,
  Transaction_ReversalKind.REVERSAL_INSUFFICIENT_FUNDS,
  Transaction_ReversalKind.REVERSAL_MANDATE_INVALID,
  Transaction_ReversalKind.REVERSAL_RETURNED,
  Transaction_ReversalKind.REVERSAL_DISPUTE,
  Transaction_ReversalKind.REVERSAL_REFUND,
  Transaction_ReversalKind.REVERSAL_OTHER,
]

export const REFUND_MODE_LABELS: Record<Transaction_RefundMode, string> = {
  [Transaction_RefundMode.ONLINE]: 'Online',
  [Transaction_RefundMode.OFFLINE]: 'Offline',
}

/** A clawed-back payment, worded by how the money left: a dispute is not a refund. */
export const reversedLabel = (
  transaction?: Pick<Transaction, 'reversalKind' | 'amountDisputed'>
): 'Charged back' | 'Reversed' | 'Refunded' => {
  if (!transaction) return 'Refunded'
  const kind = transaction.reversalKind
  if (
    transaction.amountDisputed > 0n ||
    (kind !== undefined && DISPUTE_REVERSAL_KINDS.includes(kind))
  )
    return 'Charged back'
  if (kind !== undefined && kind !== Transaction_ReversalKind.REVERSAL_REFUND) return 'Reversed'
  return 'Refunded'
}

const IN_FLIGHT_STATUSES = [
  Transaction_PaymentStatusEnum.READY,
  Transaction_PaymentStatusEnum.PENDING,
]

export const minCents = (a: bigint, b: bigint): bigint => (a < b ? a : b)

export const isRefundRow = (transaction: Transaction): boolean =>
  transaction.paymentType === Transaction_PaymentTypeEnum.REFUND

export const refundsOf = (parent: Transaction, transactions: Transaction[]): Transaction[] =>
  transactions.filter(tx => isRefundRow(tx) && tx.parentTransactionId === parent.id)

export const pendingRefundCents = (parent: Transaction, transactions: Transaction[]): bigint =>
  refundsOf(parent, transactions)
    .filter(tx => IN_FLIGHT_STATUSES.includes(tx.status))
    .reduce((sum, tx) => sum + tx.amount, 0n)

/** What is still givable back on a payment, mirroring the backend's `refundable_amount`. */
export const refundableCents = (parent: Transaction, transactions: Transaction[]): bigint => {
  const left =
    parent.amount -
    parent.amountRefunded -
    parent.amountReversed -
    pendingRefundCents(parent, transactions)
  return left > 0n ? left : 0n
}

export const isRefundableParent = (transaction: Transaction): boolean =>
  transaction.paymentType === Transaction_PaymentTypeEnum.PAYMENT &&
  transaction.status === Transaction_PaymentStatusEnum.SETTLED

/** The settled payment a refund draws on by default: the most recent one (LIFO). */
export const latestSettledPayment = (transactions: Transaction[]): Transaction | undefined =>
  transactions
    .filter(isRefundableParent)
    .sort((a, b) => (a.processedAt ?? '').localeCompare(b.processedAt ?? ''))
    .at(-1)
