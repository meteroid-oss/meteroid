import { create } from '@bufbuild/protobuf'
import { parseISO } from 'date-fns'
import { customAlphabet } from 'nanoid'

import { parseCustomProperties } from '@/features/customproperties/CustomPropertiesFields'
import { hasTerm, optionCell } from '@/features/plans/currencies/priceCells'
import { feeTypeFromPrice } from '@/features/pricing'
import { buildDefaults } from '@/features/pricing/formData'
import { emptyQuoteV2State, quoteToV2State } from '@/features/quotes/editor/quoteV2'
import { byPosition, optionHandle } from '@/features/subscriptions/pricecomponents/planOption'
import {
  ComponentOverride,
  ComponentParameterization,
  ExtraComponent,
} from '@/pages/tenants/subscription/create/state'
import { FeeStructure_BillingType, PriceSchema } from '@/rpc/api/prices/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import {
  ActivationCondition,
  SubscriptionFeeBillingPeriod,
} from '@/rpc/api/subscriptions/v1/models_pb'

import type { GrantDraft } from '@/features/entitlements/rows/grantDraft'
import type { QuoteFormValues } from '@/features/quotes/editor/quoteFormSchema'
import type { QuoteV2State } from '@/features/quotes/editor/quoteV2'
import type {
  PriceComponentsState,
  UsageExampleEntry,
} from '@/features/subscriptions/pricecomponents/PriceComponentsLogic'
import type { MinimumCommitment, PlanOption } from '@/rpc/api/plans/v1/models_pb'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'
import type { DetailedQuote, QuoteComponent } from '@/rpc/api/quotes/v1/models_pb'
import type { SubscriptionFee } from '@/rpc/api/subscriptions/v1/models_pb'

const toBillingPeriod = (period: SubscriptionFeeBillingPeriod): BillingPeriod => {
  switch (period) {
    case SubscriptionFeeBillingPeriod.QUARTERLY:
      return BillingPeriod.QUARTERLY
    case SubscriptionFeeBillingPeriod.SEMIANNUAL:
      return BillingPeriod.SEMIANNUAL
    case SubscriptionFeeBillingPeriod.YEARLY:
      return BillingPeriod.ANNUAL
    default:
      return BillingPeriod.MONTHLY
  }
}

export function subscriptionFeeToPrice(
  fee: SubscriptionFee | undefined,
  period: SubscriptionFeeBillingPeriod
): Price | undefined {
  const cadence = toBillingPeriod(period)
  switch (fee?.fee.case) {
    case 'rate':
      return create(PriceSchema, {
        cadence,
        pricing: { case: 'ratePricing', value: { rate: fee.fee.value.rate } },
      })
    case 'slot':
      return create(PriceSchema, {
        cadence,
        pricing: {
          case: 'slotPricing',
          value: {
            unitRate: fee.fee.value.unitRate,
            minSlots: fee.fee.value.minSlots,
            maxSlots: fee.fee.value.maxSlots,
            model: fee.fee.value.model,
          },
        },
      })
    case 'capacity':
      return create(PriceSchema, {
        cadence,
        pricing: {
          case: 'capacityPricing',
          value: {
            rate: fee.fee.value.rate,
            included: fee.fee.value.included,
            overageRate: fee.fee.value.overageRate,
          },
        },
      })
    case 'usage':
      return create(PriceSchema, {
        cadence,
        pricing: { case: 'usagePricing', value: { model: fee.fee.value.model } },
      })
    case 'recurring':
      return create(PriceSchema, {
        cadence,
        pricing: {
          case: 'extraRecurringPricing',
          value: {
            unitPrice: fee.fee.value.rate,
            quantity: fee.fee.value.quantity,
            model: fee.fee.value.model,
          },
        },
      })
    case 'oneTime':
      return create(PriceSchema, {
        cadence,
        pricing: {
          case: 'oneTimePricing',
          value: {
            unitPrice: fee.fee.value.rate,
            quantity: fee.fee.value.quantity,
            model: fee.fee.value.model,
          },
        },
      })
    default:
      return undefined
  }
}

// Fields that live on the product (fee structure) rather than the price, needed to recreate an
// inline extra fee.
const structuralFormData = (fee: SubscriptionFee | undefined): Record<string, unknown> => {
  switch (fee?.fee.case) {
    case 'usage':
      return { metricId: fee.fee.value.metricId }
    case 'capacity':
      return { metricId: fee.fee.value.metricId }
    case 'slot':
      return {
        slotUnitName: fee.fee.value.unit,
        quantityId: fee.fee.value.quantityId,
        upgradePolicy: 'PRORATED',
        downgradePolicy: 'REMOVE_AT_END_OF_PERIOD',
        minimumCount: fee.fee.value.minSlots,
      }
    case 'recurring':
      return {
        billingType:
          fee.fee.value.billingType === FeeStructure_BillingType.ADVANCE ? 'ADVANCE' : 'ARREAR',
      }
    default:
      return {}
  }
}

const toParameterization = (
  component: QuoteComponent,
  planComponent: PriceComponent
): ComponentParameterization | undefined => {
  const fee = component.fee?.fee
  const param: ComponentParameterization = { componentId: planComponent.id }

  if (fee?.case === 'slot') param.initialSlotCount = fee.value.initialSlots
  // Levels, not prices: each option prices the same level apart.
  const levels = new Set(
    planComponent.prices.flatMap(p =>
      p.pricing.case === 'capacityPricing' ? [p.pricing.value.included] : []
    )
  )
  if (fee?.case === 'capacity' && levels.size > 1) {
    param.committedCapacity = fee.value.included
  }

  const { componentId: _, ...values } = param
  return Object.values(values).some(v => v !== undefined) ? param : undefined
}

/**
 * A quote read back names no option: the one whose cells bill its plan lines on their stored
 * periods, else none (the editor then shows the default one sold in the quote's currency).
 */
const quotedOption = (
  quote: DetailedQuote,
  planComponents: PriceComponent[],
  options: PlanOption[]
): string | undefined => {
  const currency = quote.quote?.currency
  if (!currency || options.length < 2) return undefined
  const lines = quote.components.flatMap(c => {
    const planComponent = planComponents.find(pc => pc.id === c.priceComponentId)
    return planComponent && !c.isOverride && c.period !== SubscriptionFeeBillingPeriod.ONE_TIME
      ? [{ planComponent, cadence: toBillingPeriod(c.period) }]
      : []
  })
  const match = byPosition(options).find(option =>
    lines.every(({ planComponent, cadence }) =>
      optionCell(planComponent, option.id, currency).some(p => !hasTerm(p) || p.cadence === cadence)
    )
  )
  return match && optionHandle(match)
}

export function quoteToPriceComponentsState(
  quote: DetailedQuote,
  planComponents: PriceComponent[],
  options: PlanOption[] = []
): PriceComponentsState {
  const parameterized: ComponentParameterization[] = []
  const overridden: ComponentOverride[] = []
  const extra: ExtraComponent[] = []
  const usageExamples: UsageExampleEntry[] = []

  for (const component of quote.components) {
    const planComponent = planComponents.find(pc => pc.id === component.priceComponentId)
    // The stored fee of a discounted line is the net one; the editor reapplies the discount.
    const listFee =
      component.discountPercent && component.listFee ? component.listFee : component.fee
    const price = subscriptionFeeToPrice(listFee, component.period)

    if (component.exampleUsageQuantity) {
      usageExamples.push(
        planComponent
          ? { componentId: planComponent.id, quantity: component.exampleUsageQuantity }
          : { name: component.name, quantity: component.exampleUsageQuantity }
      )
    }

    if (planComponent && component.isOverride && price) {
      const feeType = feeTypeFromPrice(price)
      overridden.push({
        componentId: planComponent.id,
        name: component.name,
        feeType,
        formData: buildDefaults(feeType, price, true),
        productId: component.productId,
      })
      // A custom seat price keeps its seat count beside it, as the editor sets it.
      if (listFee?.fee.case === 'slot') {
        parameterized.push({
          componentId: planComponent.id,
          initialSlotCount: listFee.fee.value.initialSlots,
        })
      }
    } else if (planComponent) {
      const param = toParameterization(component, planComponent)
      if (param) parameterized.push(param)
    } else if (price) {
      const feeType = feeTypeFromPrice(price)
      extra.push({
        name: component.name,
        feeType,
        formData: { ...structuralFormData(listFee), ...buildDefaults(feeType, price) },
        productId: component.productId,
      })
    }
  }

  const keptIds = new Set(quote.components.map(c => c.priceComponentId).filter(Boolean))

  return {
    option: quotedOption(quote, planComponents, options),
    components: {
      removed: planComponents.filter(pc => !keptIds.has(pc.id)).map(pc => pc.id),
      parameterized,
      overridden,
      extra,
      usageExamples,
    },
  }
}

const activationConditionToString = (
  condition: ActivationCondition
): 'ON_START' | 'ON_CHECKOUT' | 'MANUAL' => {
  switch (condition) {
    case ActivationCondition.ON_CHECKOUT:
      return 'ON_CHECKOUT'
    case ActivationCondition.MANUAL:
      return 'MANUAL'
    default:
      return 'ON_START'
  }
}

const parseDate = (value?: string): Date | undefined => (value ? parseISO(value) : undefined)

export function quoteToFormValues(detailed: DetailedQuote): Partial<QuoteFormValues> {
  const quote = detailed.quote
  const paymentMethods = quote?.paymentMethodsConfig?.config.case

  return {
    quote_number: quote?.quoteNumber ?? '',
    customer_id: quote?.customerId ?? '',
    plan_version_id: quote?.planVersionId ?? '',
    start_date: parseDate(quote?.startDate),
    end_date: parseDate(quote?.endDate),
    trial_duration: quote?.trialDuration,
    trial_free: quote?.trialFree,
    trial_plan_id: quote?.trialPlanId,
    billing_day_anchor: quote?.billingDayAnchor,
    expires_at: parseDate(quote?.expiresAt),
    valid_until: parseDate(quote?.validUntil),
    internal_notes: quote?.internalNotes,
    net_terms: quote?.netTerms,
    recipients: quote?.recipients.length
      ? quote.recipients.map(r => ({ name: r.name, email: r.email }))
      : [{ name: '', email: '' }],
    activation_condition: activationConditionToString(
      quote?.activationCondition ?? ActivationCondition.ON_START
    ),
    payment_methods_type:
      paymentMethods === 'bankTransfer' || paymentMethods === 'external'
        ? paymentMethods
        : ('online' as const),
    auto_advance_invoices: quote?.autoAdvanceInvoices ?? true,
    charge_automatically: quote?.chargeAutomatically ?? true,
    invoice_memo: quote?.invoiceMemo,
    create_subscription_on_acceptance: quote?.createSubscriptionOnAcceptance ?? false,
    customProperties: parseCustomProperties(quote?.customProperties),
  }
}

const nanoid = customAlphabet('ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789', 10)

export const generateQuoteNumber = () =>
  `Q-${new Date().toISOString().slice(0, 10).replace(/-/g, '')}-${nanoid(5)}`

/** Everything the quote editor holds; also the payload of its autosaved drafts. */
export interface QuoteFormSeed {
  values: Partial<QuoteFormValues>
  priceComponents: PriceComponentsState
  addOns: { addOnId: string; quantity?: number }[]
  coupons: { couponId: string }[]
  minimumCommitment?: MinimumCommitment
  pendingEntitlements: GrantDraft[]
  v2: QuoteV2State
  resourceId?: string
}

export function quoteToFormSeed(
  quote: DetailedQuote,
  planComponents: PriceComponent[],
  options: PlanOption[] = []
): QuoteFormSeed {
  return {
    values: quoteToFormValues(quote),
    priceComponents: quoteToPriceComponentsState(quote, planComponents, options),
    addOns: quote.addOns.map(a => ({ addOnId: a.addOnId, quantity: a.quantity })),
    coupons: quote.coupons.map(c => ({ couponId: c.couponId })),
    minimumCommitment: quote.minimumCommitment,
    pendingEntitlements: [],
    v2: quoteToV2State(quote),
    resourceId: quote.quote?.resourceId,
  }
}

/**
 * Seed for a new quote based on an existing one: keeps the commercial terms, drops what is
 * specific to the original deal (number, customer, recipients, dates, CRM deal). Plan-bound pricing
 * is dropped too when the source plan version can no longer be subscribed to.
 */
export function quoteToTemplateSeed(
  quote: DetailedQuote,
  planComponents: PriceComponent[],
  options: { customerId: string; planVersionIsActive: boolean; planOptions?: PlanOption[] }
): QuoteFormSeed {
  const seed = quoteToFormSeed(quote, planComponents, options.planOptions)
  const values = {
    ...seed.values,
    quote_number: generateQuoteNumber(),
    customer_id: options.customerId,
    recipients: [{ name: '', email: '' }],
    start_date: undefined,
    end_date: undefined,
    expires_at: undefined,
    valid_until: undefined,
  }

  const v2 = { ...seed.v2, hubspotDealId: '' }

  if (options.planVersionIsActive) return { ...seed, values, v2, resourceId: undefined }

  return {
    values: { ...values, plan_version_id: '' },
    priceComponents: emptyPriceComponentsState(),
    addOns: [],
    coupons: seed.coupons,
    minimumCommitment: undefined,
    pendingEntitlements: [],
    v2: { ...v2, lineDetails: {}, lineOrder: undefined },
  }
}

export const emptyPriceComponentsState = (): PriceComponentsState => ({
  components: {
    removed: [],
    parameterized: [],
    overridden: [],
    extra: [],
    usageExamples: [],
  },
})

export const emptyQuoteFormSeed = (values: Partial<QuoteFormValues>): QuoteFormSeed => ({
  values,
  priceComponents: emptyPriceComponentsState(),
  addOns: [],
  coupons: [],
  pendingEntitlements: [],
  v2: emptyQuoteV2State(),
})
