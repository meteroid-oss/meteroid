import Decimal from 'decimal.js'

import {
  DetailedInvoice,
  InvoicePaymentStatus,
  LineItem,
  LineItemKind,
  Transaction_PaymentStatusEnum,
  Transaction_PaymentTypeEnum,
} from '@/rpc/api/invoices/v1/models_pb'

// What the credit-note dialog may offer, mirroring the server's refusals in
// `store/repositories/credit_notes.rs` so an operator is never led to a choice that fails.

export type Scope = 'partial' | 'full'
export type Disposition = 'reduce-debt' | 'refund' | 'credit-to-balance'

type InvoiceState = Pick<
  DetailedInvoice,
  'paymentStatus' | 'transactions' | 'appliedCredits' | 'amountDue' | 'total'
>

/** A line that sold credit: once paid, that credit is the customer's. */
export const sellsCredit = (line: Pick<LineItem, 'kind' | 'creditGrantAmount' | 'subtotal'>) =>
  line.kind === LineItemKind.CREDIT && (line.creditGrantAmount ?? 0n) > 0n && line.subtotal > 0n

// PAYMENT is every collection: the API maps top-ups and automatic top-ups to it.
export const paymentInFlight = (invoice: Pick<InvoiceState, 'paymentStatus' | 'transactions'>) =>
  invoice.paymentStatus === InvoicePaymentStatus.PROCESSING ||
  invoice.transactions.some(
    tx =>
      tx.paymentType === Transaction_PaymentTypeEnum.PAYMENT &&
      (tx.status === Transaction_PaymentStatusEnum.PENDING ||
        tx.status === Transaction_PaymentStatusEnum.READY)
  )

const isPaid = (invoice: InvoiceState) => invoice.paymentStatus === InvoicePaymentStatus.PAID
const isPartiallyPaid = (invoice: InvoiceState) =>
  invoice.paymentStatus === InvoicePaymentStatus.PARTIALLY_PAID
const isUnpaid = (invoice: InvoiceState) => !isPaid(invoice) && !isPartiallyPaid(invoice)

export type FullScopeBlocker = 'partially-paid' | 'credit-paid-part'

/**
 * Cancelling the whole invoice cancels its total as debt, which the server refuses above what is
 * still due, and refuses credit-to-balance on an invoice that is not paid.
 */
export const fullScopeBlocker = (invoice: InvoiceState): FullScopeBlocker | null => {
  if (isPartiallyPaid(invoice)) return 'partially-paid'
  if (isUnpaid(invoice) && invoice.appliedCredits > 0n && invoice.amountDue < invoice.total)
    return 'credit-paid-part'
  return null
}

/**
 * While a payment is in flight the server refuses a debt cancellation (the charge would collect
 * the full amount anyway) and accepts a credit-to-balance note, which waits for the payment.
 */
export const allowedDispositions = (invoice: InvoiceState, scope: Scope): Disposition[] => {
  if (!isPaid(invoice) && paymentInFlight(invoice)) {
    if (isUnpaid(invoice)) return ['credit-to-balance']
    return scope === 'partial' ? ['refund', 'credit-to-balance'] : []
  }
  if (isUnpaid(invoice)) return ['reduce-debt']
  if (isPaid(invoice)) return ['refund', 'credit-to-balance']
  return scope === 'partial' ? ['reduce-debt', 'refund', 'credit-to-balance'] : []
}

/** Credit-to-balance on a credit sale would grant the credit twice; the server refuses it. */
export const creditToBalanceBlocked = (creditedLines: LineItem[]) => creditedLines.some(sellsCredit)

/**
 * A refund of sold credit is capped at the credit still unspent, so a full cancellation may end
 * short of the total, and the corrected draft needs a fully credited invoice.
 */
export const reissueBlocked = (scope: Scope, disposition: Disposition, creditedLines: LineItem[]) =>
  scope === 'full' && disposition === 'refund' && creditedLines.some(sellsCredit)

export type QuantityError =
  | { kind: 'not-positive' }
  | { kind: 'above-original'; original: string }
  | { kind: 'above-remaining' }

/** A credited quantity: positive, at most what the line billed, and within what is left. */
export const quantityError = (
  quantity: string,
  originalQuantity: string,
  creditedSubunits: number,
  remainingSubunits: number
): QuantityError | null => {
  if (quantity.trim() === '') return null
  let q: Decimal
  try {
    q = new Decimal(quantity.trim())
  } catch {
    return { kind: 'not-positive' }
  }
  if (!q.isFinite() || q.lte(0)) return { kind: 'not-positive' }
  try {
    if (q.gt(new Decimal(originalQuantity)))
      return { kind: 'above-original', original: originalQuantity }
  } catch {
    // An unparsable original quantity bounds nothing.
  }
  if (creditedSubunits > remainingSubunits) return { kind: 'above-remaining' }
  return null
}

/**
 * What a note crediting `credited` (minor units, before discounts and tax) of `line` totals, tax
 * included: its taxed base and tax prorated half-up as the server prorates them. Earlier notes'
 * rounding is not replayed, so the server stays the final check.
 */
export const estimatedNoteTotal = (
  line: Pick<LineItem, 'subtotal' | 'taxableAmount' | 'taxAmount'>,
  credited: number
): bigint => {
  if (line.subtotal === 0n || credited <= 0) return 0n
  const share = (amount: bigint) =>
    BigInt(
      new Decimal(amount.toString())
        .times(credited)
        .div(line.subtotal.toString())
        .toDecimalPlaces(0, Decimal.ROUND_HALF_UP)
        .toFixed(0)
    )
  return share(line.taxableAmount) + share(line.taxAmount)
}

/** A debt cancellation cannot exceed what is still due, tax included. */
export const exceedsAmountDue = (
  disposition: Disposition,
  estimatedTotal: bigint,
  invoice: Pick<InvoiceState, 'amountDue'>
) => disposition === 'reduce-debt' && estimatedTotal > invoice.amountDue
