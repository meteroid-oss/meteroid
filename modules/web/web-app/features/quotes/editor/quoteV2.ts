import { create } from '@bufbuild/protobuf'

import {
  QuoteCountersignerSchema,
  QuoteCustomerActionsSchema,
  QuoteDisplayOptionsSchema,
  QuoteLineDetailSchema,
  QuoteSectionPlacement,
  QuoteSectionSchema,
  QuoteSignatureLevel,
} from '@/rpc/api/quotes/v1/models_pb'

import type { PriceComponentsState } from '@/features/subscriptions/pricecomponents/PriceComponentsLogic'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { DetailedQuote, QuoteLineDetail } from '@/rpc/api/quotes/v1/models_pb'

/** `c:<price component id>`, `x:<extra component name>` or `a:<add-on id>`. */
export type LineKey = string

export const lineKeys = {
  component: (priceComponentId: string): LineKey => `c:${priceComponentId}`,
  extra: (name: string): LineKey => `x:${name}`,
  addOn: (addOnId: string): LineKey => `a:${addOnId}`,
}

export interface LineDetailState {
  description: string
  discountPercent: string
}

export interface SectionState {
  title: string
  body: string
  placement: QuoteSectionPlacement
}

export interface CountersignerState {
  name: string
  email: string
  title: string
  automatic: boolean
}

export interface CustomerActionsState {
  questions: boolean
  changeRequests: boolean
  decline: boolean
}

export const ALL_CUSTOMER_ACTIONS: CustomerActionsState = {
  questions: true,
  changeRequests: true,
  decline: true,
}

const allCustomerActions = (actions: CustomerActionsState) =>
  actions.questions && actions.changeRequests && actions.decline

/** A file attached to the quote, as uploaded: the editor keeps it and sends its id. */
export interface AttachmentState {
  id: string
  filename: string
  sizeBytes: number
  sha256: string
}

export interface QuoteV2State {
  templateId?: string
  lineDetails: Record<LineKey, LineDetailState>
  /** Set once the user reorders the lines: every line is then sent with its position. */
  lineOrder?: LineKey[]
  displayOptions: { hideZeroLines: boolean; showListPrice: boolean }
  sections: SectionState[]
  signatureLevel: QuoteSignatureLevel
  consentText: string
  countersigners: CountersignerState[]
  hubspotDealId: string
  customerActions: CustomerActionsState
  attachments: AttachmentState[]
}

export const emptyQuoteV2State = (): QuoteV2State => ({
  lineDetails: {},
  displayOptions: { hideZeroLines: false, showListPrice: false },
  sections: [],
  signatureLevel: QuoteSignatureLevel.SIMPLE,
  consentText: '',
  countersigners: [],
  hubspotDealId: '',
  customerActions: { ...ALL_CUSTOMER_ACTIONS },
  attachments: [],
})

export const OVERVIEW_TITLE = 'Overview'
export const TERMS_TITLE = 'Terms & services'

/** What a new quote or template starts with: an overview before the pricing, terms after it. */
export const newQuoteV2State = (): QuoteV2State => ({
  ...emptyQuoteV2State(),
  sections: [
    { title: OVERVIEW_TITLE, body: '', placement: QuoteSectionPlacement.BEFORE_PRICING },
    { title: TERMS_TITLE, body: '', placement: QuoteSectionPlacement.AFTER_PRICING },
  ],
})

/**
 * The legacy plain-text overview and terms as sections: the overview first before the pricing,
 * the terms last after it. Empty ones add nothing.
 */
export function withLegacyDocument(
  sections: SectionState[],
  overview: string | undefined,
  terms: string | undefined
): SectionState[] {
  return [
    ...(overview?.trim()
      ? [{ title: OVERVIEW_TITLE, body: overview, placement: QuoteSectionPlacement.BEFORE_PRICING }]
      : []),
    ...sections,
    ...(terms?.trim()
      ? [{ title: TERMS_TITLE, body: terms, placement: QuoteSectionPlacement.AFTER_PRICING }]
      : []),
  ]
}

export function quoteToV2State(detailed: DetailedQuote): QuoteV2State {
  const quote = detailed.quote
  const lines = [
    ...detailed.components.map(c => ({
      key: c.priceComponentId ? lineKeys.component(c.priceComponentId) : lineKeys.extra(c.name),
      description: c.description,
      discountPercent: c.discountPercent,
      sortOrder: c.sortOrder,
    })),
    ...detailed.addOns.map(a => ({
      key: lineKeys.addOn(a.addOnId),
      description: a.description,
      discountPercent: a.discountPercent,
      sortOrder: a.sortOrder,
    })),
  ]

  const lineDetails: Record<LineKey, LineDetailState> = {}
  for (const line of lines) {
    if (line.description || line.discountPercent) {
      lineDetails[line.key] = {
        description: line.description ?? '',
        discountPercent: line.discountPercent ?? '',
      }
    }
  }

  const position = (sortOrder: number) => (sortOrder > 0 ? sortOrder : Number.MAX_SAFE_INTEGER)
  const lineOrder = lines.some(l => l.sortOrder > 0)
    ? [...lines].sort((a, b) => position(a.sortOrder) - position(b.sortOrder)).map(l => l.key)
    : undefined

  return {
    templateId: quote?.templateId,
    lineDetails,
    lineOrder,
    displayOptions: {
      hideZeroLines: quote?.displayOptions?.hideZeroLines ?? false,
      showListPrice: quote?.displayOptions?.showListPrice ?? false,
    },
    sections: withLegacyDocument(
      (quote?.sections ?? []).map(s => ({
        title: s.title,
        body: s.body,
        placement: s.placement,
      })),
      quote?.overview,
      quote?.termsAndServices
    ),
    signatureLevel: quote?.signatureLevel ?? QuoteSignatureLevel.SIMPLE,
    consentText: quote?.consentText ?? '',
    countersigners: (quote?.countersigners ?? []).map(c => ({
      name: c.name,
      email: c.email,
      title: c.title ?? '',
      automatic: c.automatic,
    })),
    hubspotDealId: quote?.hubspotDealId ?? '',
    customerActions: quote?.customerActions
      ? {
          questions: quote.customerActions.questions,
          changeRequests: quote.customerActions.changeRequests,
          decline: quote.customerActions.decline,
        }
      : { ...ALL_CUSTOMER_ACTIONS },
    attachments: (quote?.attachments ?? []).map(a => ({
      id: a.id,
      filename: a.filename,
      sizeBytes: Number(a.sizeBytes),
      sha256: a.sha256,
    })),
  }
}

export interface EditorLine {
  key: LineKey
  name: string
}

/** The priced lines of the editor, in catalog order: kept plan components, extras, add-ons. */
export function editorLines(
  planComponents: PriceComponent[],
  pricing: PriceComponentsState,
  addOns: { addOnId: string }[],
  availableAddOns: { id: string; name: string }[]
): EditorLine[] {
  const { removed, overridden, extra } = pricing.components
  return [
    ...planComponents
      .filter(pc => !removed.includes(pc.id))
      .map(pc => ({
        key: lineKeys.component(pc.id),
        name: overridden.find(o => o.componentId === pc.id)?.name ?? pc.name,
      })),
    ...extra.map(e => ({ key: lineKeys.extra(e.name), name: e.name })),
    ...addOns.map(a => ({
      key: lineKeys.addOn(a.addOnId),
      name: availableAddOns.find(x => x.id === a.addOnId)?.name ?? 'Add-on',
    })),
  ]
}

export function orderLines<T extends { key: LineKey }>(lines: T[], order?: LineKey[]): T[] {
  if (!order) return lines
  const rank = (key: LineKey) => {
    const index = order.indexOf(key)
    return index === -1 ? order.length : index
  }
  return [...lines].sort((a, b) => rank(a.key) - rank(b.key))
}

export function discountError(value: string): string | undefined {
  if (value.trim() === '') return undefined
  const discount = Number(value)
  return Number.isFinite(discount) && discount > 0 && discount <= 100
    ? undefined
    : 'A discount is above 0 and at most 100'
}

const lineTarget = (key: LineKey): QuoteLineDetail['target'] => {
  const value = key.slice(2)
  switch (key.slice(0, 2)) {
    case 'c:':
      return { case: 'priceComponentId', value }
    case 'x:':
      return { case: 'extraComponentName', value }
    default:
      return { case: 'addOnId', value }
  }
}

export function buildLineDetails(v2: QuoteV2State, orderedLines: EditorLine[]): QuoteLineDetail[] {
  const reordered = Boolean(v2.lineOrder)
  return orderedLines.flatMap((line, index) => {
    const detail = v2.lineDetails[line.key]
    const description = detail?.description.trim() || undefined
    const discountPercent = detail?.discountPercent.trim() || undefined
    if (!description && !discountPercent && !reordered) return []
    return [
      create(QuoteLineDetailSchema, {
        target: lineTarget(line.key),
        description,
        discountPercent,
        sortOrder: reordered ? index + 1 : 0,
      }),
    ]
  })
}

export function v2ToCreateQuote(v2: QuoteV2State, orderedLines: EditorLine[]) {
  return {
    templateId: v2.templateId,
    lineDetails: buildLineDetails(v2, orderedLines),
    displayOptions: create(QuoteDisplayOptionsSchema, v2.displayOptions),
    // An empty section reads as nothing: the pre-added Overview stays out until written.
    sections: v2.sections
      .filter(s => s.body.trim())
      .map(s => create(QuoteSectionSchema, { ...s, title: s.title.trim() })),
    signatureLevel: v2.signatureLevel,
    consentText: v2.consentText.trim() || undefined,
    countersigners: v2.countersigners
      .filter(c => c.name.trim() || c.email.trim())
      .map(c =>
        create(QuoteCountersignerSchema, {
          name: c.name.trim(),
          email: c.email.trim(),
          title: c.title.trim() || undefined,
          automatic: c.automatic,
        })
      ),
    hubspotDealId: v2.hubspotDealId.trim() || undefined,
    customerActions: allCustomerActions(v2.customerActions)
      ? undefined
      : create(QuoteCustomerActionsSchema, v2.customerActions),
    attachments: v2.attachments.map(a => a.id),
  }
}

/** What the quote already carries, so its controls stay visible without the entitlement. */
export const v2Presence = (v2: QuoteV2State) => ({
  lineEditing:
    Object.keys(v2.lineDetails).length > 0 ||
    Boolean(v2.lineOrder) ||
    v2.displayOptions.hideZeroLines ||
    v2.displayOptions.showListPrice,
  verified: v2.signatureLevel === QuoteSignatureLevel.VERIFIED,
  countersigners: v2.countersigners.length > 0,
  hubspot: v2.hubspotDealId.trim() !== '',
  customerActions: !allCustomerActions(v2.customerActions),
  attachments: v2.attachments.length > 0,
})

export type V2Presence = ReturnType<typeof v2Presence>
