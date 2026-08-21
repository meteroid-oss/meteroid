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
   * Threaded into the setup intent so hosted-redirect providers (GoCardless)
   * can charge the invoice after the mandate is created. Absent for checkout.
   */
  invoiceId?: string
  /**
   * Ids of this invoice's transactions already FAILED before the customer leaves
   * for a hosted (GoCardless) mandate flow. Snapshotted on departure so the
   * return handler can distinguish a genuinely new charge failure from these.
   * Only meaningful alongside `invoiceId` (invoice-payment page).
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
   * Checkout-only (no invoiceId): starts the hosted GoCardless flow that
   * authorises the mandate AND collects the first payment in one step, then
   * redirects (never resolves). When set and no mandate is saved, the direct
   * debit tab renders a single pay button instead of pre-fetching a setup intent.
   */
  onHostedDirectDebit?: (connectionId: string) => Promise<void>
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
