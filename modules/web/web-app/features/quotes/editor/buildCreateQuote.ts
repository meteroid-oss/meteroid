import { create } from '@bufbuild/protobuf'

import { serializeCustomProperties } from '@/features/customproperties/CustomPropertiesFields'
import {
  buildExistingProductRef,
  buildFeeStructure,
  buildNewProductRef,
  buildPriceInputs,
  toPricingTypeFromFeeType,
  wrapAsNewPriceEntries,
} from '@/features/pricing'
import { v2ToCreateQuote } from '@/features/quotes/editor/quoteV2'
import { trialEndToProto } from '@/features/quotes/quoteTrial'
import { mapDatev2 } from '@/lib/mapping'
import {
  CreateQuoteCouponSchema,
  CreateQuoteCouponsSchema,
  CreateQuoteSchema,
  QuoteUsageExampleSchema,
} from '@/rpc/api/quotes/v1/models_pb'
import {
  ActivationCondition,
  BankTransferSchema,
  CreateSubscriptionAddOnSchema,
  CreateSubscriptionAddOnsSchema,
  CreateSubscriptionComponentsSchema,
  CreateSubscriptionComponents_ComponentOverrideSchema,
  CreateSubscriptionComponents_ExtraComponentSchema,
  ExternalSchema,
  OnlinePaymentSchema,
  PaymentMethodsConfigSchema,
} from '@/rpc/api/subscriptions/v1/models_pb'

import type { QuoteFormValues } from '@/features/quotes/editor/quoteFormSchema'
import type { EditorLine, QuoteV2State } from '@/features/quotes/editor/quoteV2'
import type { PriceComponentsState } from '@/features/subscriptions/pricecomponents/PriceComponentsLogic'
import type { EntitlementSpec } from '@/rpc/api/entitlements/v1/models_pb'
import type { MinimumCommitment } from '@/rpc/api/plans/v1/models_pb'
import type { CreateQuote } from '@/rpc/api/quotes/v1/models_pb'
import type { PaymentMethodsConfig } from '@/rpc/api/subscriptions/v1/models_pb'

const activationConditionFromString = (
  condition: QuoteFormValues['activation_condition']
): ActivationCondition => {
  switch (condition) {
    case 'ON_CHECKOUT':
      return ActivationCondition.ON_CHECKOUT
    case 'MANUAL':
      return ActivationCondition.MANUAL
    default:
      return ActivationCondition.ON_START
  }
}

const buildPaymentMethodsConfig = (
  type: QuoteFormValues['payment_methods_type']
): PaymentMethodsConfig => {
  switch (type) {
    case 'bankTransfer':
      return create(PaymentMethodsConfigSchema, {
        config: { case: 'bankTransfer', value: create(BankTransferSchema) },
      })
    case 'external':
      return create(PaymentMethodsConfigSchema, {
        config: { case: 'external', value: create(ExternalSchema) },
      })
    default:
      return create(PaymentMethodsConfigSchema, {
        config: { case: 'online', value: create(OnlinePaymentSchema) },
      })
  }
}

export interface QuoteEditorPayload {
  values: QuoteFormValues
  currency: string
  /** The plan option quoted (its alias), as the lines show it. */
  option?: string
  pricing: PriceComponentsState
  addOns: { addOnId: string; quantity?: number }[]
  coupons: { couponId: string }[]
  minimumOverride?: MinimumCommitment
  v2: QuoteV2State
  orderedLines: EditorLine[]
  entitlements: EntitlementSpec[]
  /** A stored resource of the customer the subscription will be for. */
  resourceId?: string
}

/** The `CreateQuote` of the editor state: what Create, Update and the totals preview send. */
export function buildCreateQuote({
  values: data,
  currency,
  option,
  pricing,
  addOns,
  coupons,
  minimumOverride,
  v2,
  orderedLines,
  entitlements,
  resourceId,
}: QuoteEditorPayload): CreateQuote {
  const { parameterized, overridden, extra, removed, usageExamples } = pricing.components

  const components = create(CreateSubscriptionComponentsSchema, {
    // A draft saved before options may still carry a per-charge `billingPeriod`: the option rules.
    // A seat count beside a custom price is sent too: the override takes it.
    parameterizedComponents: parameterized.map(p => ({
      componentId: p.componentId,
      initialSlotCount: p.initialSlotCount,
      committedCapacity: p.committedCapacity,
    })),
    overriddenComponents: overridden.map(c => {
      const pricingType = toPricingTypeFromFeeType(
        c.feeType,
        c.formData.usageModel as string | undefined
      )
      return create(CreateSubscriptionComponents_ComponentOverrideSchema, {
        componentId: c.componentId,
        name: c.name,
        price: wrapAsNewPriceEntries(
          buildPriceInputs(pricingType, c.formData as Record<string, unknown>, currency)
        )[0],
      })
    }),
    extraComponents: extra.map(c => {
      const pricingType = toPricingTypeFromFeeType(
        c.feeType,
        c.formData.usageModel as string | undefined
      )
      return create(CreateSubscriptionComponents_ExtraComponentSchema, {
        name: c.name,
        product: c.productId ? buildExistingProductRef(c.productId) : buildNewProductRef(c.name),
        feeStructure: buildFeeStructure(c.feeType, c.formData as Record<string, unknown>),
        price: wrapAsNewPriceEntries(
          buildPriceInputs(pricingType, c.formData as Record<string, unknown>, currency)
        )[0],
      })
    }),
    removeComponents: removed,
  })

  return create(CreateQuoteSchema, {
    quoteNumber: data.quote_number,
    planVersionId: data.plan_version_id,
    customerId: data.customer_id,
    currency,
    option,
    startDate: data.start_date ? mapDatev2(data.start_date) : undefined,
    billingStartDate: data.billing_start_date ? mapDatev2(data.billing_start_date) : undefined,
    endDate: data.end_date ? mapDatev2(data.end_date) : undefined,
    trialDuration: data.trial_duration,
    trialPlanId: data.trial_duration ? data.trial_plan_id : undefined,
    trialEndAction: trialEndToProto(data.trial_duration, data.trial_end_action),
    billingDayAnchor: data.billing_day_anchor,
    expiresAt: data.expires_at ? mapDatev2(data.expires_at) : undefined,
    validUntil: data.valid_until ? mapDatev2(data.valid_until) : undefined,
    internalNotes: data.internal_notes,
    netTerms: data.net_terms,
    activationCondition: activationConditionFromString(data.activation_condition),
    paymentMethodsConfig: buildPaymentMethodsConfig(data.payment_methods_type),
    autoAdvanceInvoices: data.auto_advance_invoices,
    chargeAutomatically: data.charge_automatically,
    invoiceMemo: data.invoice_memo,
    createSubscriptionOnAcceptance: data.create_subscription_on_acceptance,
    customProperties: serializeCustomProperties(data.customProperties),
    recipients: data.recipients.map(r => ({ name: r.name, email: r.email })),
    components,
    addOns: create(CreateSubscriptionAddOnsSchema, {
      addOns: addOns.map(a =>
        create(CreateSubscriptionAddOnSchema, { addOnId: a.addOnId, quantity: a.quantity ?? 1 })
      ),
    }),
    coupons: create(CreateQuoteCouponsSchema, {
      coupons: coupons.map(c => create(CreateQuoteCouponSchema, { couponId: c.couponId })),
    }),
    entitlements,
    minimumCommitmentOverride: minimumOverride,
    resourceId,
    usageExamples: (usageExamples ?? [])
      .filter(e => e.quantity.trim() !== '')
      .map(e =>
        create(QuoteUsageExampleSchema, {
          priceComponentId: e.componentId,
          componentName: e.componentId ? undefined : e.name,
          exampleQuantity: e.quantity,
        })
      ),
    ...v2ToCreateQuote(v2, orderedLines),
  })
}

const wholeNumber = (value: unknown, min: number, max = Number.MAX_SAFE_INTEGER) =>
  typeof value === 'number' && Number.isInteger(value) && value >= min && value <= max
    ? value
    : undefined

/**
 * The form as it stands, before validation, made sendable for the totals preview: a field still
 * being typed (an emptied number) counts as unset, as the server's default would apply.
 */
export const previewFormValues = (values: Partial<QuoteFormValues>): QuoteFormValues => ({
  quote_number: values.quote_number ?? '',
  customer_id: values.customer_id ?? '',
  plan_version_id: values.plan_version_id ?? '',
  start_date: values.start_date,
  billing_start_date: values.billing_start_date,
  end_date: values.end_date,
  trial_duration: wholeNumber(values.trial_duration, 0),
  trial_plan_id: values.trial_plan_id,
  trial_end_action: values.trial_end_action,
  billing_day_anchor: wholeNumber(values.billing_day_anchor, 1, 31),
  expires_at: values.expires_at,
  valid_until: values.valid_until,
  net_terms: wholeNumber(values.net_terms, 0),
  recipients: (values.recipients ?? []).filter(r => (r?.email ?? '').trim() !== ''),
  activation_condition: values.activation_condition ?? 'ON_START',
  payment_methods_type: values.payment_methods_type ?? 'online',
  auto_advance_invoices: values.auto_advance_invoices ?? true,
  charge_automatically: values.charge_automatically ?? true,
  create_subscription_on_acceptance: values.create_subscription_on_acceptance ?? false,
})
