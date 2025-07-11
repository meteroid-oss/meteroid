import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import { ActivationCondition } from '@/rpc/api/subscriptions/v1/models_pb'
import { atom } from 'jotai'

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
  fee: any // Will be typed based on SubscriptionFee
}

export interface ExtraComponent {
  name: string
  fee: any // Will be typed based on SubscriptionFee
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
    fee: any
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
  netTerms: number
  invoiceMemo?: string
  invoiceThreshold?: string

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

export const createSubscriptionAtom = atom<CreateSubscriptionState>({
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
  netTerms: 30,
  invoiceMemo: undefined,
  invoiceThreshold: undefined,

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
