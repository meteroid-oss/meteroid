import { Customer, CustomerPaymentMethod } from '@/rpc/api/customers/v1/models_pb'
import { Checkout } from '@/rpc/portal/checkout/v1/models_pb'

/**
 * Payment method selection types
 */
export type SavedPaymentMethodSelection = {
  type: 'saved'
  id: string
}

export type NewPaymentMethodSelection = {
  type: 'new'
  methodType: 'card' | 'bank'
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
  onPaymentSubmit: (paymentMethodId: string, isNew: boolean) => Promise<void>
}

/**
 * Props for the CheckoutFlow component
 */
export interface CheckoutFlowProps {
  checkoutData: Checkout
}
