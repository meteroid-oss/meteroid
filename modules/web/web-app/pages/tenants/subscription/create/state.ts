
import { atomWithReset } from 'jotai/utils'

import { formDataToPrice } from '@/features/pricing'
import { MinimumCommitment } from '@/rpc/api/plans/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import { ActivationCondition } from '@/rpc/api/subscriptions/v1/models_pb'

import type { GrantDraft } from '@/features/entitlements/rows/grantDraft'
import type { ComponentFeeType } from '@/features/pricing'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'

export type PaymentMethodsConfigType = 'online' | 'bankTransfer' | 'external'

// Component configuration types
// A plan charge's cadence is its option's (`CreateSubscriptionState.option`), not chosen per charge.
export interface ComponentParameterization {
  componentId: string
  initialSlotCount?: number
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
  quantity?: number
  parameterization?: {
    initialSlotCount?: number
    billingPeriod?: BillingPeriod
    committedCapacity?: bigint
  }
}

export interface CreateSubscriptionCoupon {
  couponId: string
}

/**
 * The prices a subscription or quote will bill: the plan's components (as bought in its option, see
 * `inOption`) minus the removed ones, with overrides applied, plus extras and the add-ons' prices.
 */
export const effectiveSubscriptionPrices = (
  planComponents: PriceComponent[],
  components: Pick<
    CreateSubscriptionState['components'],
    'removed' | 'parameterized' | 'overridden' | 'extra'
  >,
  addOnPrices: (Price | undefined)[],
  currency: string
): Price[] => {
  const fromForm = (c: { feeType: ComponentFeeType; formData: Record<string, unknown> }) => {
    try {
      return [formDataToPrice(c.feeType, c.formData, currency)]
    } catch {
      return []
    }
  }
  const planPrices = planComponents
    .filter(pc => !components.removed.includes(pc.id))
    .flatMap(pc => {
      const override = components.overridden.find(o => o.componentId === pc.id)
      if (override) return fromForm(override)
      // The one price the server resolves (`select_price`): narrowed by the chosen capacity;
      // still ambiguous, the component is not resolvable yet and counts for nothing.
      const params = components.parameterized.find(p => p.componentId === pc.id)
      const candidates = pc.prices.filter(
        p =>
          params?.committedCapacity === undefined ||
          p.pricing.case !== 'capacityPricing' ||
          p.pricing.value.included === params.committedCapacity
      )
      return candidates.length === 1 ? candidates : []
    })
  return [
    ...planPrices,
    ...components.extra.flatMap(fromForm),
    ...addOnPrices.filter((p): p is Price => p !== undefined),
  ]
}

export interface CreateSubscriptionState {
  // Basic info
  customerId?: string
  planVersionId?: string
  // The plan option bought (its alias); unset: the default one sold in the customer's currency.
  option?: string

  // Timing & billing
  startDate: Date
  endDate?: Date
  billingDayAnchor?: number
  billingDay: 'FIRST' | 'SUB_START_DAY'
  trialDuration?: number

  // Advanced settings
  activationCondition: ActivationCondition
  paymentMethodsType: PaymentMethodsConfigType
  // Unset unless typed in: the server then applies its default (due on receipt when selling credit).
  netTerms?: number
  invoiceMemo?: string
  invoiceThreshold?: string
  purchaseOrder?: string
  // The stored resource the subscription is for; unset for the whole customer.
  resourceId?: string
  autoAdvanceInvoices: boolean
  chargeAutomatically: boolean
  skipPastInvoices: boolean

  // JSON-serializable custom property values keyed by definition key.
  customProperties?: Record<string, unknown>

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

  // Entitlements
  entitlements: GrantDraft[]

  // Optional operator-set minimum-commitment override applied at creation.
  minimumOverride?: MinimumCommitment
  // Drop the plan's minimum-commitment floor for this subscription (only when no override is set).
  dropMinimum?: boolean
}

export const createSubscriptionAtom = atomWithReset<CreateSubscriptionState>({
  customerId: undefined,
  planVersionId: undefined,
  option: undefined,

  startDate: new Date(),
  endDate: undefined,
  billingDayAnchor: undefined,
  billingDay: 'SUB_START_DAY',
  trialDuration: undefined,

  activationCondition: ActivationCondition.ON_START,
  paymentMethodsType: 'online',
  netTerms: undefined,
  invoiceMemo: undefined,
  invoiceThreshold: undefined,
  purchaseOrder: undefined,
  resourceId: undefined,
  autoAdvanceInvoices: true,
  chargeAutomatically: true,
  skipPastInvoices: false,
  customProperties: undefined,

  components: {
    parameterized: [],
    overridden: [],
    extra: [],
    removed: []
  },

  addOns: [],
  coupons: [],
  entitlements: [],
  minimumOverride: undefined,
  dropMinimum: false
})

/** A resource only belongs to its customer: the URL's goes with the URL's customer, and switching
 * customers drops it. */
export const resourceFor = (
  customerId: string,
  state: { customerId?: string; resourceId?: string },
  customerIdFromUrl: string | null,
  resourceIdFromUrl: string | null
): string | undefined => {
  if (state.customerId && customerId === state.customerId) return state.resourceId
  return customerId === customerIdFromUrl ? (resourceIdFromUrl ?? undefined) : undefined
}
