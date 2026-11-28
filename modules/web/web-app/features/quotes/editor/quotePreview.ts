import { create } from '@bufbuild/protobuf'
import { useMemo } from 'react'

import { formDataToPrice } from '@/features/pricing'
import { lineKeys, v2ToCreateQuote } from '@/features/quotes/editor/quoteV2'
import { trialEndToProto } from '@/features/quotes/quoteTrial'
import { useQuery } from '@/lib/connectrpc'
import { mapDatev2 } from '@/lib/mapping'
import {
  getPrice,
  priceToSubscriptionFee,
  priceToSubscriptionPeriod,
} from '@/lib/mapping/priceToSubscriptionFee'
import { previewPrice } from '@/rpc/api/prices/v1/prices-PricesService_connectquery'
import { PreviewPriceItemSchema } from '@/rpc/api/prices/v1/prices_pb'
import {
  DetailedQuoteSchema,
  QuoteAttachmentSchema,
  QuoteCouponTermsSchema,
  QuoteSchema,
} from '@/rpc/api/quotes/v1/models_pb'

import type { QuoteLine } from '@/features/quotes/detail/QuoteLinesTable'
import type { QuoteFormValues } from '@/features/quotes/editor/quoteFormSchema'
import type { EditorLine, LineKey, QuoteV2State } from '@/features/quotes/editor/quoteV2'
import type { PriceComponentsState } from '@/features/subscriptions/pricecomponents/PriceComponentsLogic'
import type { AddOn } from '@/rpc/api/addons/v1/models_pb'
import type { Coupon } from '@/rpc/api/coupons/v1/models_pb'
import type { CreditUnit } from '@/rpc/api/creditunits/v1/models_pb'
import type { Customer } from '@/rpc/api/customers/v1/models_pb'
import type { InvoicingEntity } from '@/rpc/api/invoicingentities/v1/models_pb'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'
import type { PreviewPriceItem } from '@/rpc/api/prices/v1/prices_pb'
import type { DetailedQuote } from '@/rpc/api/quotes/v1/models_pb'

export interface PreviewLine extends QuoteLine {
  key: LineKey
}

const bigintSafe = (_: string, v: unknown) => (typeof v === 'bigint' ? v.toString() : v)

/**
 * The priced lines as the document shows them, with the money amount of each usage example
 * computed by the server.
 */
export function useQuotePreviewLines({
  planComponents,
  pricing,
  currency,
  creditUnits,
}: {
  planComponents: PriceComponent[]
  pricing: PriceComponentsState
  currency?: string
  creditUnits?: CreditUnit[]
}) {
  const { lines, items } = useMemo(() => {
    if (!currency) return { lines: [] as PreviewLine[], items: [] as PreviewPriceItem[] }

    const { parameterized, overridden, extra, removed, usageExamples } = pricing.components
    const exampleFor = (key: { componentId?: string; name?: string }): string | undefined =>
      (usageExamples ?? []).find(e =>
        key.componentId ? e.componentId === key.componentId : e.name === key.name
      )?.quantity

    const previewItems: PreviewPriceItem[] = []

    const makeLine = (
      key: LineKey,
      id: string,
      name: string,
      price: Price,
      quantity: string | undefined,
      feeOptions?: { initialSlotCount?: number }
    ): PreviewLine => {
      // An example in credits has no money amount to preview.
      if (
        price.pricing.case === 'usagePricing' &&
        !price.creditUnit &&
        price.pricing.value.model.case &&
        price.pricing.value.model.case !== 'dimensionRates' &&
        price.pricing.value.model.case !== 'rateCard' &&
        quantity &&
        quantity.trim() !== ''
      ) {
        previewItems.push(
          create(PreviewPriceItemSchema, {
            key: id,
            usagePricing: price.pricing.value,
            quantity,
            currency,
          })
        )
      }
      return {
        key,
        id,
        name,
        period: priceToSubscriptionPeriod(price),
        fee: priceToSubscriptionFee(price, { ...feeOptions, creditUnits }),
        exampleUsageQuantity: quantity,
      }
    }

    const defaults = planComponents
      .filter(
        pc =>
          !removed.includes(pc.id) &&
          !parameterized.some(c => c.componentId === pc.id) &&
          !overridden.some(c => c.componentId === pc.id)
      )
      .flatMap(pc => {
        const price = getPrice(pc)
        if (!price) return []
        return [
          makeLine(
            lineKeys.component(pc.id),
            pc.id,
            pc.name,
            price,
            exampleFor({ componentId: pc.id })
          ),
        ]
      })

    // A seat count kept beside a custom price is the override's line, below.
    const configured = parameterized.flatMap(c => {
      const pc = planComponents.find(pc => pc.id === c.componentId)
      if (!pc || overridden.some(o => o.componentId === pc.id)) return []
      const price = getPrice(pc)
      if (!price) return []
      return [
        makeLine(
          lineKeys.component(pc.id),
          pc.id,
          pc.name,
          price,
          exampleFor({ componentId: pc.id }),
          { initialSlotCount: c.initialSlotCount }
        ),
      ]
    })

    const overrides = overridden.map(c =>
      makeLine(
        lineKeys.component(c.componentId),
        c.componentId,
        c.name,
        formDataToPrice(c.feeType, c.formData as Record<string, unknown>, currency),
        exampleFor({ componentId: c.componentId }),
        {
          initialSlotCount: parameterized.find(p => p.componentId === c.componentId)
            ?.initialSlotCount,
        }
      )
    )

    const extras = extra.map(c =>
      makeLine(
        lineKeys.extra(c.name),
        c.name,
        c.name,
        formDataToPrice(c.feeType, c.formData as Record<string, unknown>, currency),
        exampleFor({ name: c.name })
      )
    )

    return { lines: [...defaults, ...configured, ...overrides, ...extras], items: previewItems }
  }, [planComponents, pricing, currency, creditUnits])

  // uint64 fields in the request are BigInt, which react-query's default key
  // hashing can't serialize — provide a BigInt-safe key.
  const previewPriceQuery = useQuery(
    previewPrice,
    { items },
    {
      enabled: items.length > 0,
      queryKeyHashFn: () => 'previewPrice-' + JSON.stringify(items, bigintSafe),
    }
  )

  const exampleAmountByKey = useMemo(
    () => new Map((previewPriceQuery.data?.results ?? []).map(r => [r.key, r.amount])),
    [previewPriceQuery.data]
  )

  const previewLines = useMemo(
    () =>
      lines.map(line =>
        exampleAmountByKey.has(line.id)
          ? { ...line, exampleUsageAmount: exampleAmountByKey.get(line.id) }
          : line
      ),
    [lines, exampleAmountByKey]
  )

  return { previewLines, exampleAmountByKey }
}

interface PreviewQuoteInput {
  values: Partial<QuoteFormValues>
  currency?: string
  lines: PreviewLine[]
  orderedLines: EditorLine[]
  v2: QuoteV2State
  addOns: { addOnId: string; quantity?: number }[]
  availableAddOns: AddOn[]
  coupons: { couponId: string }[]
  availableCoupons: Coupon[]
  creditUnits?: CreditUnit[]
  /** Shown when the net terms are left to the server's default. */
  defaultNetTerms?: number
  customer?: Customer
  invoicingEntity?: InvoicingEntity
}

/** The unsaved quote as the customer will read it. Line discounts are only applied on save. */
export function buildPreviewQuote({
  values: data,
  currency,
  lines,
  orderedLines,
  v2,
  addOns,
  availableAddOns,
  coupons,
  availableCoupons,
  creditUnits,
  defaultNetTerms,
  customer,
  invoicingEntity,
}: PreviewQuoteInput): { quote: DetailedQuote; lines: QuoteLine[] } {
  const v2Fields = v2ToCreateQuote(v2, orderedLines)
  const position = new Map(orderedLines.map((line, index) => [line.key, index + 1]))
  const presentation = (key: LineKey) => ({
    description: v2.lineDetails[key]?.description.trim() || undefined,
    sortOrder: v2.lineOrder ? (position.get(key) ?? 0) : 0,
  })

  const quote = create(QuoteSchema, {
    id: 'preview-quote',
    quoteNumber: data.quote_number,
    planVersionId: data.plan_version_id,
    customerId: data.customer_id,
    currency,
    startDate: data.start_date ? mapDatev2(data.start_date) : mapDatev2(new Date()),
    billingStartDate: data.billing_start_date ? mapDatev2(data.billing_start_date) : undefined,
    endDate: data.end_date ? mapDatev2(data.end_date) : undefined,
    trialDuration: typeof data.trial_duration === 'number' ? data.trial_duration : undefined,
    trialFree: data.trial_duration ? true : undefined,
    trialPlanId: data.trial_duration ? data.trial_plan_id : undefined,
    trialEndAction: trialEndToProto(data.trial_duration, data.trial_end_action),
    billingDayAnchor:
      typeof data.billing_day_anchor === 'number' ? data.billing_day_anchor : undefined,
    expiresAt: data.expires_at ? mapDatev2(data.expires_at) : undefined,
    validUntil: data.valid_until ? mapDatev2(data.valid_until) : undefined,
    internalNotes: data.internal_notes,
    netTerms: typeof data.net_terms === 'number' ? data.net_terms : defaultNetTerms,
    recipients: (data.recipients ?? []).map(r => ({ name: r.name, email: r.email })),
    createdAt: new Date().toISOString(),
    templateId: v2Fields.templateId,
    displayOptions: v2Fields.displayOptions,
    sections: v2Fields.sections,
    signatureLevel: v2Fields.signatureLevel,
    consentText: v2Fields.consentText,
    countersigners: v2Fields.countersigners,
    customerActions: v2Fields.customerActions,
    attachments: v2.attachments.map(a =>
      create(QuoteAttachmentSchema, {
        id: a.id,
        filename: a.filename,
        contentType: 'application/pdf',
        sizeBytes: BigInt(a.sizeBytes),
        sha256: a.sha256,
      })
    ),
  })

  const addOnItems = addOns.flatMap(sel => {
    const addOn = availableAddOns.find(a => a.id === sel.addOnId)
    if (!addOn?.price) return []
    return [
      {
        id: addOn.id,
        addOnId: addOn.id,
        name: addOn.name,
        quantity: sel.quantity ?? 1,
        period: priceToSubscriptionPeriod(addOn.price),
        fee: priceToSubscriptionFee(addOn.price, { creditUnits }),
        ...presentation(lineKeys.addOn(addOn.id)),
      },
    ]
  })

  const couponTerms = coupons.flatMap(({ couponId }) => {
    const coupon = availableCoupons.find(c => c.id === couponId)
    if (!coupon) return []
    return [
      create(QuoteCouponTermsSchema, {
        code: coupon.code,
        description: coupon.description.trim() || undefined,
        discount: coupon.discount,
        recurringValue: coupon.recurringValue,
        durationMonths: coupon.durationMonths,
      }),
    ]
  })

  return {
    quote: create(DetailedQuoteSchema, {
      quote,
      addOns: addOnItems,
      couponTerms,
      customer,
      invoicingEntity,
    }),
    lines: lines.map(line => ({ ...line, ...presentation(line.key) })),
  }
}
