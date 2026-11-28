import { formatCurrencyNoRounding } from '@/lib/utils/numbers'
import {
  GrantAddOn_AtEnd,
  GrantCredit_Scope,
  GrantWindow_Anchor,
  SubscriptionGrant_Source,
  SubscriptionGrant_Status,
} from '@/rpc/api/offers/v1/offers_pb'

import type { EntitlementEffect, EntitlementValue } from '@/rpc/api/entitlements/v1/models_pb'
import type { Grant, GrantNewSchema, GrantWindow, GrantWindowSchema } from '@/rpc/api/offers/v1/offers_pb'
import type { MessageInitShape } from '@bufbuild/protobuf'

export type GrantKind = 'discount' | 'credit' | 'entitlement' | 'addOn'
export type WindowAnchor = 'start' | 'billing'
export type WindowDuration = 'forever' | 'days' | 'months' | 'untilBilling' | 'invoices'

export interface WindowForm {
  anchor: WindowAnchor
  duration: WindowDuration
  /** Days, months or invoices; unset for `forever` and `untilBilling`. */
  count?: number
}

export interface AmountRow {
  currency: string
  amount: string
}

export type DiscountValue =
  | { type: 'percentage'; percentage: string }
  | { type: 'fixed'; currency: string; amount: string; otherAmounts: AmountRow[] }

interface GrantBase {
  /** A key for lists; grants are replaced on save, never patched. */
  key: string
  window: WindowForm
}

export type DiscountGrantForm = GrantBase & {
  kind: 'discount'
  discount: DiscountValue
  addOnIds: string[]
}
export type CreditGrantForm = GrantBase & {
  kind: 'credit'
  currency: string
  amount: string
  scope: 'customer' | 'subscription'
}
export type EntitlementGrantForm = GrantBase & {
  kind: 'entitlement'
  featureId: string
  value?: EntitlementValue
  effect: EntitlementEffect
}
export type AddOnGrantForm = GrantBase & {
  kind: 'addOn'
  addOnId: string
  quantity: number
  atEnd: 'revert' | 'convert'
}

export type GrantForm = DiscountGrantForm | CreditGrantForm | EntitlementGrantForm | AddOnGrantForm

export type GrantDefinition<K extends GrantKind> = K extends GrantKind
  ? Omit<Extract<GrantForm, { kind: K }>, 'key' | 'window'>
  : never

export const GRANT_KIND_LABELS: Record<GrantKind, string> = {
  discount: 'Discount',
  credit: 'Credit',
  entitlement: 'Feature',
  addOn: 'Add-on',
}

/** The windows each kind's engine supports (ADR 0019 §2); `invoices` is read only. */
export const WINDOW_OPTIONS: Record<
  GrantKind,
  { anchors: WindowAnchor[]; durations: Exclude<WindowDuration, 'invoices'>[] }
> = {
  discount: { anchors: ['billing'], durations: ['months', 'forever'] },
  credit: { anchors: ['start', 'billing'], durations: ['days', 'months', 'forever'] },
  entitlement: {
    anchors: ['start', 'billing'],
    durations: ['days', 'months', 'untilBilling', 'forever'],
  },
  addOn: { anchors: ['start'], durations: ['months', 'untilBilling'] },
}

export const DEFAULT_WINDOWS: Record<GrantKind, WindowForm> = {
  discount: { anchor: 'billing', duration: 'months', count: 3 },
  credit: { anchor: 'start', duration: 'days', count: 30 },
  entitlement: { anchor: 'start', duration: 'days', count: 30 },
  addOn: { anchor: 'start', duration: 'months', count: 1 },
}

export const DURATION_LABELS: Record<WindowDuration, string> = {
  days: 'Days',
  months: 'Months',
  untilBilling: 'Until the trial ends',
  forever: 'Forever',
  invoices: 'Invoices',
}

export const ANCHOR_LABELS: Record<WindowAnchor, string> = {
  start: 'The start',
  billing: 'Billing start',
}

export const hasCount = (duration: WindowDuration) =>
  duration === 'days' || duration === 'months' || duration === 'invoices'

/** Whether the window is one the kind supports, with a count where it needs one. */
export const windowIssue = (kind: GrantKind, window: WindowForm): string | undefined => {
  const options = WINDOW_OPTIONS[kind]
  if (!options.anchors.includes(window.anchor)) return 'Not available for this grant'
  if (window.duration === 'untilBilling' && window.anchor !== 'start') {
    return 'Counted from the start'
  }
  if (window.duration !== 'invoices' && !options.durations.includes(window.duration)) {
    return 'Not available for this grant'
  }
  if (hasCount(window.duration)) {
    const count = window.count ?? 0
    if (!Number.isInteger(count) || count < 1) return 'Enter at least 1'
    if (window.duration === 'days' && count > 3650) return 'At most 3650 days'
    if (window.duration === 'months' && count > 120) return 'At most 120 months'
  }
  return undefined
}

const plural = (count: number, one: string, many: string) =>
  `${count} ${count === 1 ? one : many}`

/** "30 days from start", "3 months from billing", "until the trial ends", "forever". */
export const windowLabel = (window: WindowForm): string => {
  const from = window.anchor === 'start' ? 'from start' : 'from billing'
  const count = window.count ?? 0
  switch (window.duration) {
    case 'forever':
      return 'forever'
    case 'untilBilling':
      return 'until the trial ends'
    case 'days':
      return `${plural(count, 'day', 'days')} ${from}`
    case 'months':
      return `${plural(count, 'month', 'months')} ${from}`
    case 'invoices':
      return `first ${plural(count, 'invoice', 'invoices')}`
  }
}

/** The window in a chip: "30 d", "3 months", "trial", "forever". */
export const shortWindowLabel = (window: WindowForm): string => {
  const count = window.count ?? 0
  switch (window.duration) {
    case 'forever':
      return 'forever'
    case 'untilBilling':
      return 'trial'
    case 'days':
      return `${count} d`
    case 'months':
      return plural(count, 'month', 'months')
    case 'invoices':
      return plural(count, 'invoice', 'invoices')
  }
}

export const windowToProto = (window: WindowForm): MessageInitShape<typeof GrantWindowSchema> => {
  const anchor = window.anchor === 'start' ? GrantWindow_Anchor.START : GrantWindow_Anchor.BILLING_START
  const count = window.count ?? 0
  switch (window.duration) {
    case 'forever':
      return { anchor, duration: { case: 'forever', value: true } }
    case 'untilBilling':
      return { anchor, duration: { case: 'untilBillingStarts', value: true } }
    case 'days':
      return { anchor, duration: { case: 'days', value: count } }
    case 'months':
      return { anchor, duration: { case: 'months', value: count } }
    case 'invoices':
      return { anchor, duration: { case: 'invoices', value: count } }
  }
}

export const windowFromProto = (window: GrantWindow | undefined): WindowForm => {
  const anchor: WindowAnchor = window?.anchor === GrantWindow_Anchor.BILLING_START ? 'billing' : 'start'
  const duration = window?.duration
  switch (duration?.case) {
    case 'days':
      return { anchor, duration: 'days', count: duration.value }
    case 'months':
      return { anchor, duration: 'months', count: duration.value }
    case 'invoices':
      return { anchor, duration: 'invoices', count: duration.value }
    case 'untilBillingStarts':
      return { anchor, duration: 'untilBilling' }
    default:
      return { anchor, duration: 'forever' }
  }
}

let keySeed = 0
export const newGrantKey = () => `grant-${Date.now().toString(36)}-${(keySeed++).toString(36)}`

export const grantToProto = (grant: GrantForm): MessageInitShape<typeof GrantNewSchema> => {
  const window = windowToProto(grant.window)
  switch (grant.kind) {
    case 'discount': {
      const d = grant.discount
      return {
        window,
        definition: {
          case: 'discount',
          value:
            d.type === 'percentage'
              ? { discount: { case: 'percentage', value: d.percentage.trim() }, addOnIds: grant.addOnIds }
              : {
                  discount: {
                    case: 'fixed',
                    value: { currency: d.currency, amount: d.amount.trim() },
                  },
                  fixedAmounts: d.otherAmounts
                    .map(row => ({ currency: row.currency, amount: row.amount.trim() }))
                    .filter(row => row.currency && row.amount && row.currency !== d.currency),
                  addOnIds: grant.addOnIds,
                },
        },
      }
    }
    case 'credit':
      return {
        window,
        definition: {
          case: 'credit',
          value: {
            currency: grant.currency,
            amount: grant.amount.trim(),
            scope:
              grant.scope === 'subscription'
                ? GrantCredit_Scope.SUBSCRIPTION
                : GrantCredit_Scope.CUSTOMER,
          },
        },
      }
    case 'entitlement':
      return {
        window,
        definition: {
          case: 'entitlement',
          value: { featureId: grant.featureId, value: grant.value, effect: grant.effect },
        },
      }
    case 'addOn':
      return {
        window,
        definition: {
          case: 'addOn',
          value: {
            addOnId: grant.addOnId,
            quantity: grant.quantity,
            atEnd: grant.atEnd === 'convert' ? GrantAddOn_AtEnd.CONVERT : GrantAddOn_AtEnd.REVERT,
          },
        },
      }
  }
}

/** A stored grant as the editor holds it; one without a definition is left out. */
export const grantFromProto = (grant: Grant): GrantForm | undefined => {
  const base = { key: grant.id || newGrantKey(), window: windowFromProto(grant.window) }
  const def = grant.definition
  switch (def.case) {
    case 'discount': {
      const inner = def.value.discount
      const discount: DiscountValue =
        inner.case === 'fixed'
          ? {
              type: 'fixed',
              currency: inner.value.currency,
              amount: inner.value.amount,
              otherAmounts: def.value.fixedAmounts.map(a => ({ currency: a.currency, amount: a.amount })),
            }
          : { type: 'percentage', percentage: inner.case === 'percentage' ? inner.value : '' }
      return { ...base, kind: 'discount', discount, addOnIds: [...def.value.addOnIds] }
    }
    case 'credit':
      return {
        ...base,
        kind: 'credit',
        currency: def.value.currency,
        amount: def.value.amount,
        scope: def.value.scope === GrantCredit_Scope.SUBSCRIPTION ? 'subscription' : 'customer',
      }
    case 'entitlement':
      return {
        ...base,
        kind: 'entitlement',
        featureId: def.value.featureId,
        value: def.value.value,
        effect: def.value.effect,
      }
    case 'addOn':
      return {
        ...base,
        kind: 'addOn',
        addOnId: def.value.addOnId,
        quantity: def.value.quantity || 1,
        atEnd: def.value.atEnd === GrantAddOn_AtEnd.CONVERT ? 'convert' : 'revert',
      }
    default:
      return undefined
  }
}

export const grantsFromProto = (grants: Grant[]): GrantForm[] =>
  grants.flatMap(grant => {
    const form = grantFromProto(grant)
    return form ? [form] : []
  })

/** Names the catalog items a grant refers to; unknown ones read as their kind. */
export interface GrantNames {
  feature?: (id: string) => string | undefined
  addOn?: (id: string) => string | undefined
  /** The value as the entitlements screens print it, when the feature is loaded. */
  entitlementValue?: (featureId: string, value: EntitlementValue | undefined) => string | undefined
}

const money = (amount: string, currency: string) => {
  const parsed = Number(amount)
  return Number.isFinite(parsed) && currency ? formatCurrencyNoRounding(amount, currency) : `${amount} ${currency}`.trim()
}

/** "enabled", "10,000", "unlimited": the value when its feature isn't loaded. */
export const plainEntitlementValue = (value: EntitlementValue | undefined): string => {
  const inner = value?.value
  switch (inner?.case) {
    case 'booleanValue':
      return inner.value.enabled ? 'enabled' : 'disabled'
    case 'meteredValue':
      if (!inner.value.enabled) return 'disabled'
      return inner.value.limit ? Number(inner.value.limit).toLocaleString('en-US') : 'unlimited'
    case 'configValue': {
      const config = inner.value.value
      if (config?.case === 'boolean') return config.value ? 'on' : 'off'
      return config?.value !== undefined ? String(config.value) : 'set'
    }
    default:
      return 'enabled'
  }
}

const discountText = (grant: DiscountGrantForm) => {
  const d = grant.discount
  const off = d.type === 'percentage' ? `${d.percentage.trim()}% off` : `${money(d.amount, d.currency)} off`
  const scope =
    grant.addOnIds.length === 0
      ? undefined
      : grant.addOnIds.length === 1
        ? '1 add-on'
        : `${grant.addOnIds.length} add-ons`
  return scope ? `${off} · ${scope}` : off
}

/** What the grant gives: "20% off", "$50.00 credit", "SSO · enabled", "AI × 2 · free". */
export const grantDefinitionLabel = (grant: GrantForm, names: GrantNames = {}): string => {
  switch (grant.kind) {
    case 'discount':
      return discountText(grant)
    case 'credit': {
      const credit = `${money(grant.amount, grant.currency)} credit`
      return grant.scope === 'subscription' ? `${credit} · this subscription` : credit
    }
    case 'entitlement': {
      const name = names.feature?.(grant.featureId) ?? 'Feature'
      const value =
        names.entitlementValue?.(grant.featureId, grant.value) ?? plainEntitlementValue(grant.value)
      return `${name} · ${value}`
    }
    case 'addOn': {
      const name = names.addOn?.(grant.addOnId) ?? 'Add-on'
      const quantity = grant.quantity > 1 ? ` × ${grant.quantity}` : ''
      return `${name}${quantity} · free${grant.atEnd === 'convert' ? ', then paid' : ''}`
    }
  }
}

export type ChipTone = 'info' | 'success' | 'warning' | 'secondary'

export interface GrantChip {
  key: string
  label: string
  tone: ChipTone
}

export const KIND_TONES: Record<GrantKind, Exclude<ChipTone, 'info'>> = {
  credit: 'success',
  entitlement: 'warning',
  discount: 'secondary',
  addOn: 'secondary',
}

/** A grant at a glance: "Credit $50.00 · 30 d", "SSO · 30 d", "20% off · 3 months". */
export const grantChip = (grant: GrantForm, names: GrantNames = {}): GrantChip => {
  const window = shortWindowLabel(grant.window)
  const label = (() => {
    switch (grant.kind) {
      case 'discount':
        return `${discountText(grant)} · ${window}`
      case 'credit':
        return `Credit ${money(grant.amount, grant.currency)} · ${window}`
      case 'entitlement':
        return `${names.feature?.(grant.featureId) ?? 'Feature'} · ${window}`
      case 'addOn': {
        const name = names.addOn?.(grant.addOnId)
        return `${name ? `Add-on ${name}` : 'Add-on'} · free ${window}`
      }
    }
  })()
  return { key: grant.key, label, tone: KIND_TONES[grant.kind] }
}

export const SUBSCRIPTION_GRANT_STATUS: Record<
  SubscriptionGrant_Status,
  { label: string; tone: 'info' | 'success' | 'secondary' | 'outline' }
> = {
  [SubscriptionGrant_Status.SCHEDULED]: { label: 'Scheduled', tone: 'info' },
  [SubscriptionGrant_Status.ACTIVE]: { label: 'Active', tone: 'success' },
  [SubscriptionGrant_Status.ENDED]: { label: 'Ended', tone: 'secondary' },
  [SubscriptionGrant_Status.WITHDRAWN]: { label: 'Withdrawn', tone: 'outline' },
}

export const GRANT_SOURCE_LABELS: Record<SubscriptionGrant_Source, string> = {
  [SubscriptionGrant_Source.PLAN]: 'Plan',
  [SubscriptionGrant_Source.OPERATOR]: 'Given',
  [SubscriptionGrant_Source.COUPON]: 'Coupon',
  [SubscriptionGrant_Source.RETENTION]: 'Retention',
  [SubscriptionGrant_Source.QUOTE]: 'Quote',
  [SubscriptionGrant_Source.API]: 'API',
}

/** The kinds an editor offers: add-ons only where allowed, features only with entitlements on. */
export const grantKinds = ({ addOns, features }: { addOns: boolean; features: boolean }): GrantKind[] =>
  (['credit', 'entitlement', 'discount', 'addOn'] as const).filter(
    kind => (kind !== 'addOn' || addOns) && (kind !== 'entitlement' || features)
  )
