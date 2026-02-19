
import { atomWithReset } from 'jotai/utils'

import { Price } from '@/rpc/api/prices/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import { ActivationCondition } from '@/rpc/api/subscriptions/v1/models_pb'

import type { ComponentFeeType } from '@/features/pricing'

export type PaymentMethodsConfigType = 'online' | 'bankTransfer' | 'external'

// Component configuration types
export interface ComponentParameterization {
  componentId: string
  initialSlotCount?: number
  billingPeriod?: BillingPeriod
  committedCapacity?: bigint
}

export interface ComponentOverride {
  componentId: string
  name: string
  feeType: ComponentFeeType
  formData: Record<string, unknown>
  productId?: string
}

export interface ExtraComponent {
  name: string
  description?: string
  feeType: ComponentFeeType
  formData: Record<string, unknown>
  productId?: string
}

export interface CreateSubscriptionAddOn {
  addOnId: string
  parameterization?: {
    initialSlotCount?: number
    billingPeriod?: BillingPeriod
    committedCapacity?: bigint
  }
  override?: {
    name: string
    price: Price
  }
}

export interface CreateSubscriptionCoupon {
  couponId: string
}

export interface CreateSubscriptionState {
  // Basic info
  customerId?: string
  planVersionId?: string

  // Timing & billing
  startDate: Date
  endDate?: Date
  billingDayAnchor?: number
  billingDay: 'FIRST' | 'SUB_START_DAY'
  trialDuration?: number

  // Advanced settings
  activationCondition: ActivationCondition
  paymentMethodsType: PaymentMethodsConfigType
  netTerms: number
  invoiceMemo?: string
  invoiceThreshold?: string
  purchaseOrder?: string
  autoAdvanceInvoices: boolean
  chargeAutomatically: boolean
  skipPastInvoices: boolean

  // Components configuration
  components: {
    parameterized: ComponentParameterization[]
    overridden: ComponentOverride[]
    extra: ExtraComponent[]
    removed: string[]
  }

  // Add-ons & coupons
  addOns: CreateSubscriptionAddOn[]
  coupons: CreateSubscriptionCoupon[]
}

export const createSubscriptionAtom = atomWithReset<CreateSubscriptionState>({
  customerId: undefined,
  planVersionId: undefined,

  startDate: new Date(),
  endDate: undefined,
  billingDayAnchor: undefined,
  billingDay: 'SUB_START_DAY',
  trialDuration: undefined,

  activationCondition: ActivationCondition.ON_START,
  paymentMethodsType: 'online',
  netTerms: 30,
  invoiceMemo: undefined,
  invoiceThreshold: undefined,
  purchaseOrder: undefined,
  autoAdvanceInvoices: true,
  chargeAutomatically: true,
  skipPastInvoices: false,

  components: {
    parameterized: [],
    overridden: [],
    extra: [],
    removed: []
  },

  addOns: [],
  coupons: []
})
