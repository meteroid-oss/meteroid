import { describe, expect, it } from 'vitest'

import { Transaction_PaymentStatusEnum } from '@/rpc/api/invoices/v1/models_pb'

import {
  invoicePaymentOutcome,
  invoicePaymentPortalUrl,
  invoicePaymentSuccessUrl,
} from './paymentOutcome'

describe('invoicePaymentOutcome', () => {
  it('reports a failed transaction as declined, with its decline class', () => {
    expect(
      invoicePaymentOutcome({
        status: Transaction_PaymentStatusEnum.FAILED,
        error: 'Your card was declined for insufficient funds.',
      })
    ).toEqual({
      kind: 'declined',
      message:
        'Your card was declined for insufficient funds. Please try again or use a different payment method.',
    })
  })

  it('is a success only when settled, and processing while the provider confirms', () => {
    expect(invoicePaymentOutcome({ status: Transaction_PaymentStatusEnum.SETTLED })).toEqual({
      kind: 'success',
      processing: false,
    })
    expect(invoicePaymentOutcome({ status: Transaction_PaymentStatusEnum.PENDING })).toEqual({
      kind: 'success',
      processing: true,
    })
  })

  it('never reads an unknown or cancelled state as a success', () => {
    for (const transaction of [
      undefined,
      { status: Transaction_PaymentStatusEnum.CANCELLED },
      { status: Transaction_PaymentStatusEnum.READY },
    ]) {
      expect(invoicePaymentOutcome(transaction)).toEqual({
        kind: 'declined',
        message:
          'Your payment could not be completed. Please try again or use a different payment method.',
      })
    }
  })
})

describe('invoicePaymentSuccessUrl', () => {
  it('is the portal success route whatever page it is built from', () => {
    expect(
      invoicePaymentSuccessUrl({ invoiceNumber: 'INV-8', customerName: 'Acme', processing: true })
    ).toBe('/portal/invoice-payment/success?invoice=INV-8&customer=Acme&status=processing')
    expect(invoicePaymentSuccessUrl({ processing: false })).toBe(
      '/portal/invoice-payment/success?invoice=&customer='
    )
  })
})

describe('invoicePaymentPortalUrl', () => {
  const token = (resource: Record<string, unknown>) =>
    ['h', btoa(JSON.stringify({ resource })).replace(/=+$/, ''), 's'].join('.')

  it('leads back to the portal invoices with a customer portal token', () => {
    const portal = token({ Customer: 'cus_1' })
    expect(invoicePaymentPortalUrl(portal)).toBe(
      `/portal/customer?page=invoices&token=${encodeURIComponent(portal)}`
    )
  })

  it('offers no way back with an invoice token from an email, or none', () => {
    expect(invoicePaymentPortalUrl(token({ Invoice: 'inv_1' }))).toBeUndefined()
    expect(invoicePaymentPortalUrl(null)).toBeUndefined()
  })
})
