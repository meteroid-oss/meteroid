import { opensCustomerPortal } from '@/features/auth/portalToken'
import { Transaction_PaymentStatusEnum } from '@/rpc/api/invoices/v1/models_pb'

import type { Transaction } from '@/rpc/api/invoices/v1/models_pb'

// Absolute: a path relative to `/portal/invoice-payment/<id>` lands in the dashboard's `/:org/:tenant`.
export const INVOICE_PAYMENT_SUCCESS_PATH = '/portal/invoice-payment/success'

const RETRY = 'Please try again or use a different payment method.'

export type InvoicePaymentOutcome =
  { kind: 'declined'; message: string } | { kind: 'success'; processing: boolean }

/**
 * What a confirmed invoice payment came to. Only a settled payment is a success; one the provider
 * still confirms (direct debit, an authenticated card) is processing; anything else failed. The
 * server sends only the decline class as the transaction's error.
 */
export const invoicePaymentOutcome = (
  transaction: Pick<Transaction, 'status' | 'error'> | undefined
): InvoicePaymentOutcome => {
  switch (transaction?.status) {
    case Transaction_PaymentStatusEnum.SETTLED:
      return { kind: 'success', processing: false }
    case Transaction_PaymentStatusEnum.PENDING:
      return { kind: 'success', processing: true }
    default: {
      const reason = transaction?.error?.trim()
      return {
        kind: 'declined',
        message: reason ? `${reason} ${RETRY}` : `Your payment could not be completed. ${RETRY}`,
      }
    }
  }
}

export const invoicePaymentSuccessUrl = ({
  invoiceNumber,
  customerName,
  processing,
}: {
  invoiceNumber?: string
  customerName?: string
  processing: boolean
}) => {
  const params = new URLSearchParams({ invoice: invoiceNumber || '', customer: customerName || '' })
  if (processing) params.set('status', 'processing')
  return `${INVOICE_PAYMENT_SUCCESS_PATH}?${params.toString()}`
}

/**
 * The way back once paid: the customer's portal invoices, when the page was opened with a portal
 * token that may read the portal (from the portal). An invoice token from an email opens none.
 */
export const invoicePaymentPortalUrl = (token: string | null): string | undefined =>
  token && opensCustomerPortal(token)
    ? `/portal/customer?${new URLSearchParams({ page: 'invoices', token }).toString()}`
    : undefined
