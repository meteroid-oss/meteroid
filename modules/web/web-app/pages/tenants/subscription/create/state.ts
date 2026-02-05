
import { atomWithReset } from 'jotai/utils'

import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import { ActivationCondition } from '@/rpc/api/subscriptions/v1/models_pb'

export type PaymentMethodsConfigType = 'online' | 'bankTransfer' | 'external'

// Subscription-specific fee types (not plan fee types)
export interface SubscriptionFeeData {
  unitPrice: string
  quantity?: number
  // Slot-specific
  slotUnitName?: string
  minSlots?: number
  maxSlots?: number
  // Capacity-specific
  includedAmount?: string
  overageRate?: string
  metricId?: string
  // Rate-specific
  billingType?: 'ARREAR' | 'ADVANCE'
}

export interface SubscriptionFeeType {
  fee: 'rate' | 'oneTime' | 'extraRecurring' | 'slot' | 'capacity' | 'usage'
  data: SubscriptionFeeData
}

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
  fee: SubscriptionFeeType
}

export interface ExtraComponent {
  name: string
  fee: SubscriptionFeeType
  billingPeriod?: BillingPeriod
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
    fee: SubscriptionFeeType
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
  // Basic info
  customerId: undefined,
  planVersionId: undefined,

  // Timing & billing
  startDate: new Date(),
  endDate: undefined,
  billingDayAnchor: undefined,
  billingDay: 'SUB_START_DAY',
  trialDuration: undefined,

  // Advanced settings
  activationCondition: ActivationCondition.ON_START,
  paymentMethodsType: 'online',
  netTerms: 30,
  invoiceMemo: undefined,
  invoiceThreshold: undefined,
  purchaseOrder: undefined,
  autoAdvanceInvoices: true,
  chargeAutomatically: true,

  // Components configuration
  components: {
    parameterized: [],
    overridden: [],
    extra: [],
    removed: []
  },

  // Add-ons & coupons
  addOns: [],
  coupons: []
})
