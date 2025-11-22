import { BankAccount } from '@/rpc/api/bankaccounts/v1/models_pb'
import {
  InvoicePaymentStatus,
  InvoiceStatus,
  Transaction,
  Transaction_PaymentStatusEnum,
} from '@/rpc/api/invoices/v1/models_pb'
import { SubscriptionStatus } from '@/rpc/api/subscriptions/v1/models_pb'

/**
 * Determines what payment UI should be displayed based on configuration and state
 */
export type PaymentAvailability =
  | {
      type: 'payment_form'
      methods: ('card' | 'direct_debit' | 'bank')[]
      cardConnectionId?: string
      directDebitConnectionId?: string
      bankAccount?: BankAccount
    }
  | {
      type: 'bank_only'
      bankAccount: BankAccount
    }
  | {
      type: 'readonly'
      reason:
        | 'already_paid'
        | 'voided'
        | 'cancelled'
        | 'uncollectible'
        | 'no_payment_methods'
        | 'external_payment'
        | 'already_active'
        | 'draft_invoice'
        | 'pending_payment'
      displayTransactions?: boolean
    }

/**
 * Determines payment availability for subscription checkout
 */
export function getCheckoutPaymentAvailability(config: {
  subscriptionStatus?: SubscriptionStatus
  cardConnectionId?: string
  directDebitConnectionId?: string
  bankAccount?: BankAccount
}): PaymentAvailability {
  const { subscriptionStatus, cardConnectionId, directDebitConnectionId, bankAccount } = config

  // Check if subscription is already active or in a terminal state
  if (subscriptionStatus === SubscriptionStatus.ACTIVE) {
    return {
      type: 'readonly',
      reason: 'already_active',
    }
  }

  if (
    subscriptionStatus === SubscriptionStatus.CANCELED ||
    subscriptionStatus === SubscriptionStatus.ENDED
  ) {
    return {
      type: 'readonly',
      reason: 'cancelled',
    }
  }

  // Determine available payment methods
  const hasOnlinePayment = !!(cardConnectionId || directDebitConnectionId)
  const hasBankTransfer = !!bankAccount

  // No payment methods configured at all
  if (!hasOnlinePayment && !hasBankTransfer) {
    return {
      type: 'readonly',
      reason: 'no_payment_methods',
    }
  }

  // Only bank transfer available
  if (!hasOnlinePayment && hasBankTransfer) {
    return {
      type: 'bank_only',
      bankAccount,
    }
  }

  // Online payment available (with optional bank transfer)
  const methods: ('card' | 'direct_debit' | 'bank')[] = []
  if (cardConnectionId) methods.push('card')
  if (directDebitConnectionId) methods.push('direct_debit')
  if (hasBankTransfer) methods.push('bank')

  return {
    type: 'payment_form',
    methods,
    cardConnectionId,
    directDebitConnectionId,
    bankAccount,
  }
}

/**
 * Determines payment availability for invoice payment
 */
export function getInvoicePaymentAvailability(config: {
  invoiceStatus?: InvoiceStatus
  paymentStatus?: InvoicePaymentStatus
  cardConnectionId?: string
  directDebitConnectionId?: string
  bankAccount?: BankAccount
  hasTransactions?: boolean
  transactions?: Transaction[]
}): PaymentAvailability {
  const {
    invoiceStatus,
    paymentStatus,
    cardConnectionId,
    directDebitConnectionId,
    bankAccount,
    hasTransactions,
    transactions,
  } = config

  // Check invoice status first
  if (invoiceStatus === InvoiceStatus.VOID) {
    return {
      type: 'readonly',
      reason: 'voided',
      displayTransactions: hasTransactions,
    }
  }

  if (invoiceStatus === InvoiceStatus.UNCOLLECTIBLE) {
    return {
      type: 'readonly',
      reason: 'uncollectible',
      displayTransactions: hasTransactions,
    }
  }

  // Check payment status
  if (paymentStatus === InvoicePaymentStatus.PAID) {
    return {
      type: 'readonly',
      reason: 'already_paid',
      displayTransactions: true,
    }
  }

  // Check for pending transactions
  if (transactions && transactions.length > 0) {
    const hasPendingTransaction = transactions.some(
      tx => tx.status === Transaction_PaymentStatusEnum.PENDING
    )

    if (hasPendingTransaction) {
      return {
        type: 'readonly',
        reason: 'pending_payment',
        displayTransactions: true,
      }
    }
  }

  // Draft invoices typically shouldn't be paid via portal
  if (invoiceStatus === InvoiceStatus.DRAFT) {
    return {
      type: 'readonly',
      reason: 'draft_invoice',
    }
  }

  // Determine available payment methods
  const hasOnlinePayment = !!(cardConnectionId || directDebitConnectionId)
  const hasBankTransfer = !!bankAccount

  // No payment methods configured at all
  if (!hasOnlinePayment && !hasBankTransfer) {
    return {
      type: 'readonly',
      reason: 'external_payment',
    }
  }

  // Only bank transfer available
  if (!hasOnlinePayment && hasBankTransfer) {
    return {
      type: 'bank_only',
      bankAccount,
    }
  }

  // Online payment available (with optional bank transfer)
  const methods: ('card' | 'direct_debit' | 'bank')[] = []
  if (cardConnectionId) methods.push('card')
  if (directDebitConnectionId) methods.push('direct_debit')
  if (hasBankTransfer) methods.push('bank')

  return {
    type: 'payment_form',
    methods,
    cardConnectionId,
    directDebitConnectionId,
    bankAccount,
  }
}

/**
 * Get user-friendly message for readonly reasons
 */
export function getReadonlyMessage(reason: PaymentAvailability['type'] extends 'readonly' ? PaymentAvailability['reason'] : never): string {
  const messages: Record<string, string> = {
    already_paid: 'This invoice has been paid.',
    voided: 'This invoice has been voided and cannot be paid.',
    cancelled: 'This subscription has been cancelled.',
    uncollectible: 'This invoice has been marked as uncollectible.',
    no_payment_methods: 'No payment methods are available for this invoice.',
    external_payment: 'Payment for this invoice is handled externally. Please contact support for payment instructions.',
    already_active: 'This subscription is already active.',
    draft_invoice: 'This invoice is in draft status and cannot be paid yet.',
    pending_payment: 'A payment for this invoice is already being processed. Please wait for it to complete before attempting another payment.',
  }
  return messages[reason] || 'Payment is not available.'
}
