import { Customer, CustomerPaymentMethod } from '@/rpc/api/customers/v1/models_pb'
import {
  AddOnPurchaseCheckoutContext,
  CheckoutType,
  PlanChangeCheckoutContext,
} from '@/rpc/portal/checkout/v1/checkout_pb'
import { Checkout } from '@/rpc/portal/checkout/v1/models_pb'

import type { PortalThemeConfig } from '@/pages/portal/experience/theme'

/**
 * Payment method selection types
 */
export type SavedPaymentMethodSelection = {
  type: 'saved'
  id: string
}

export type NewPaymentMethodSelection = {
  type: 'new'
  methodType: 'card' | 'directDebit'
}

export type PaymentMethodSelection = SavedPaymentMethodSelection | NewPaymentMethodSelection

/**
 * Payment process state
 */
export enum PaymentState {
  INITIAL = 'initial',
  PROCESSING = 'processing',
  SUCCESS = 'success',
  ERROR = 'error',
}

/**
 * Props for the PaymentPanel component
 */
export interface PaymentPanelProps {
  customer?: Customer
  paymentMethods: CustomerPaymentMethod[]
  totalAmount: string
  currency: string
  cardConnectionId?: string
  directDebitConnectionId?: string
  onPaymentSubmit: (paymentMethodId: string) => Promise<void>
  /**
   * Invoice being paid, when this panel is used from the invoice-payment page.
   * Threaded into the setup intent so hosted-redirect providers can charge
   * the invoice after the mandate/card is created. Absent for checkout.
   */
  invoiceId?: string
  /**
   * Ids of this invoice's transactions already FAILED before the customer leaves
   * for a provider-hosted (GoCardless, Stancer) flow. Snapshotted on departure
   * so the return handler can distinguish a genuinely new charge failure from
   * these. Only meaningful alongside `invoiceId` (invoice-payment page).
   */
  preAttemptFailedTxIds?: string[]
  /** Resolved theme of the surrounding checkout pane, so Stripe Elements match. */
  themeConfig: PortalThemeConfig
  /**
   * Called after a new payment method is confirmed and attached (Stripe setup),
   * so the parent can refetch its payment-method list. Optional.
   */
  onPaymentMethodAttached?: () => void
  /**
   * Checkout-only (no invoiceId): starts a provider-hosted checkout flow via
   * InitiateHostedCheckout, then redirects (never resolves). When set and no
   * method is saved, the matching tab renders a single pay button.
   */
  onHostedCheckout?: (connectionId: string) => Promise<void>
  /**
   * Invoice-payment page only (with invoiceId): starts the provider-hosted
   * invoice payment via InitiateHostedInvoicePayment, then redirects (never
   * resolves). The pay CLICK is what pre-creates the transaction and mints
   * the capturing intent — rendering the page never does.
   */
  onHostedInvoicePayment?: (connectionId: string) => Promise<void>
}

/**
 * Props for the CheckoutFlow component
 */
export interface CheckoutFlowProps {
  checkoutData: Checkout
  checkoutType?: CheckoutType
  planChangeContext?: PlanChangeCheckoutContext
  addonPurchaseContext?: AddOnPurchaseCheckoutContext
}
