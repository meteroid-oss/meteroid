import { BankAccount } from '@/rpc/api/bankaccounts/v1/models_pb'
import {
  InvoicePaymentStatus,
  InvoiceStatus,
  Transaction,
  Transaction_PaymentStatusEnum,
} from '@/rpc/api/invoices/v1/models_pb'
import { SubscriptionStatus } from '@/rpc/api/subscriptions/v1/models_pb'
import { CheckoutType } from '@/rpc/portal/checkout/v1/checkout_pb'

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
  | {
      // An abandoned provider-hosted payment attempt (Stancer): the backend
      // resumes the SAME intent, so the customer may continue instead of
      // being stuck on the readonly "payment in progress" view.
      type: 'resumable_hosted_payment'
      connectionId: string
      displayTransactions?: boolean
    }

/**
 * Determines payment availability for subscription checkout
 */
export function getCheckoutPaymentAvailability(config: {
  subscriptionStatus?: SubscriptionStatus
  checkoutType?: CheckoutType
  cardConnectionId?: string
  directDebitConnectionId?: string
  bankAccount?: BankAccount
}): PaymentAvailability {
  const {
    subscriptionStatus,
    checkoutType,
    cardConnectionId,
    directDebitConnectionId,
    bankAccount,
  } = config

  // For plan changes and addon purchases, the subscription is expected to be active — skip the active check
  if (
    subscriptionStatus === SubscriptionStatus.ACTIVE &&
    checkoutType !== CheckoutType.PLAN_CHANGE &&
    checkoutType !== CheckoutType.ADDON_PURCHASE
  ) {
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

  // A payment was accepted by the provider and is awaiting settlement
  if (paymentStatus === InvoicePaymentStatus.PROCESSING) {
    return {
      type: 'readonly',
      reason: 'pending_payment',
      displayTransactions: true,
    }
  }

  // Check for pending transactions
  if (transactions && transactions.length > 0) {
    const pendingTransactions = transactions.filter(
      tx => tx.status === Transaction_PaymentStatusEnum.PENDING
    )

    if (pendingTransactions.length > 0) {
      // A Pending tx marked resumable is an abandoned hosted attempt: the
      // backend rehydrates the same intent, so offer "Continue payment".
      // A marker-less Pending tx (off-session charge in flight) stays readonly.
      const resumable = pendingTransactions.find(tx => tx.resumableHostedConnectionId)
      if (resumable?.resumableHostedConnectionId) {
        return {
          type: 'resumable_hosted_payment',
          connectionId: resumable.resumableHostedConnectionId,
          displayTransactions: true,
        }
      }
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
