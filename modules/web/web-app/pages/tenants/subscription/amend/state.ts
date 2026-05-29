import { atomWithReset } from 'jotai/utils'

import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import {
  PlanChangeApplyMode,
  PreviewAmendmentResponse,
} from '@/rpc/api/subscriptions/v1/subscriptions_pb'

import type { ComponentFeeType } from '@/features/pricing'

// --- Editor draft types ---

/**
 * Override the price of an existing live subscription component.
 * `formData` is produced by ProductPricingForm for the given `feeType`,
 * and is converted to a single PriceEntry when building the request.
 */
export interface EditComponentDraft {
  subscriptionComponentId: string
  name?: string
  feeType: ComponentFeeType
  formData: Record<string, unknown>
}

/**
 * Add an ad-hoc extra component (no plan price-component reference).
 * Mirrors the create flow's ExtraComponent.
 */
export interface ExtraComponentDraft {
  name: string
  feeType: ComponentFeeType
  formData: Record<string, unknown>
  productId?: string
}

/** Add a new add-on, mirroring the create flow's CreateSubscriptionAddOn shape. */
export interface AddedAddOnDraft {
  addOnId: string
  quantity: number
  parameterization?: {
    initialSlotCount?: number
    billingPeriod?: BillingPeriod
    committedCapacity?: bigint
  }
}

/** Edit quantity and/or price of an existing live add-on. */
export interface EditAddOnDraft {
  subscriptionAddOnId: string
  name: string
  quantity?: number
  priceOverride?: {
    feeType: ComponentFeeType
    formData: Record<string, unknown>
  }
}

export interface AmendSubscriptionState {
  subscriptionId: string
  currency: string
  applyMode: PlanChangeApplyMode

  componentChanges: {
    edited: EditComponentDraft[]
    added: ExtraComponentDraft[]
    removedComponentIds: string[]
  }

  addOnChanges: {
    added: AddedAddOnDraft[]
    edited: EditAddOnDraft[]
    removedAddOnIds: string[]
  }

  preview?: PreviewAmendmentResponse
}

export const amendSubscriptionAtom = atomWithReset<AmendSubscriptionState>({
  subscriptionId: '',
  currency: '',
  applyMode: PlanChangeApplyMode.END_OF_PERIOD,
  componentChanges: {
    edited: [],
    added: [],
    removedComponentIds: [],
  },
  addOnChanges: {
    added: [],
    edited: [],
    removedAddOnIds: [],
  },
  preview: undefined,
})
