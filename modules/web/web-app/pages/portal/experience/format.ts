import {
  CustomerPaymentMethod,
  CustomerPaymentMethod_PaymentMethodTypeEnum as PmType,
} from '@/rpc/api/customers/v1/models_pb'
import { FeeType } from '@/rpc/api/prices/v1/models_pb'
import {
  SubscriptionFeeBillingPeriod,
  SubscriptionStatus,
} from '@/rpc/api/subscriptions/v1/models_pb'

import type { ComponentFee } from '@/rpc/portal/subscription/v1/subscription_pb'

/** Format an integer amount of minor units (cents) in its currency. */
export const money = (cents: bigint | number, currency: string, opts?: { compact?: boolean }) => {
  const value = Number(cents) / 100
  try {
    return new Intl.NumberFormat('en-US', {
      style: 'currency',
      currency: currency || 'USD',
      minimumFractionDigits: opts?.compact && Number.isInteger(value) ? 0 : 2,
      maximumFractionDigits: 2,
    }).format(value)
  } catch {
    return `${currency} ${value.toFixed(2)}`
  }
}

/** Format a decimal-string amount (e.g. a component rate) in its currency. */
export const moneyStr = (amount: string, currency: string) => {
  const n = Number(amount)
  if (Number.isNaN(n)) return amount
  return money(Math.round(n * 100), currency)
}

const DATE_FMT = new Intl.DateTimeFormat('en-US', {
  month: 'short',
  day: 'numeric',
  year: 'numeric',
})

/** Format an ISO date/datetime string as "Jul 1, 2026". Empty when missing. */
export const date = (iso?: string) => {
  if (!iso) return ''
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return iso
  return DATE_FMT.format(d)
}

const PERIOD_SUFFIX: Record<SubscriptionFeeBillingPeriod, string> = {
  [SubscriptionFeeBillingPeriod.ONE_TIME]: 'one-time',
  [SubscriptionFeeBillingPeriod.MONTHLY]: '/mo',
  [SubscriptionFeeBillingPeriod.QUARTERLY]: '/qtr',
  [SubscriptionFeeBillingPeriod.YEARLY]: '/yr',
  [SubscriptionFeeBillingPeriod.SEMIANNUAL]: '/6mo',
}

export const periodSuffix = (cadence: SubscriptionFeeBillingPeriod) => PERIOD_SUFFIX[cadence] ?? ''

/**
 * Human pricing label for a headline component fee, e.g.:
 *   Rate    → "$290.00 /mo"
 *   Slot    → "$12.00 /seat /mo"
 *   Usage   → "Usage-based"
 *   Capacity→ "$0.10 /unit + base"
 */
export const feeLabel = (fee: ComponentFee | undefined, currency: string): string => {
  if (!fee) return '—'
  const suffix = periodSuffix(fee.cadence)
  switch (fee.feeType) {
    case FeeType.USAGE:
      return 'Usage-based'
    case FeeType.SLOT: {
      const unit = fee.unit ? ` /${fee.unit}` : ' /seat'
      return `${moneyStr(fee.amount, currency)}${unit} ${suffix}`.trim()
    }
    case FeeType.ONE_TIME:
      return `${moneyStr(fee.amount, currency)} one-time`
    default:
      return `${moneyStr(fee.amount, currency)} ${suffix}`.trim()
  }
}

/** Best-effort monthly headline number (in cents) for a component fee. */
export const feeMonthlyCents = (fee: ComponentFee | undefined): number => {
  if (!fee) return 0
  const n = Number(fee.amount)
  if (Number.isNaN(n)) return 0
  switch (fee.cadence) {
    case SubscriptionFeeBillingPeriod.YEARLY:
      return Math.round((n / 12) * 100)
    case SubscriptionFeeBillingPeriod.QUARTERLY:
      return Math.round((n / 3) * 100)
    case SubscriptionFeeBillingPeriod.SEMIANNUAL:
      return Math.round((n / 6) * 100)
    default:
      return Math.round(n * 100)
  }
}

export interface StatusBadge {
  label: string
  tone: 'ok' | 'neutral' | 'warn' | 'danger'
}

// Keyed by a normalized (letters-only, uppercased) status token. Covers both
// the proto SubscriptionStatus enum names and the Rust `{:?}` debug variants
// the backend emits as a string on SubscriptionDetails.
const BADGE_BY_KEY: Record<string, StatusBadge> = {
  ACTIVE: { label: 'Active', tone: 'ok' },
  TRIALING: { label: 'Trial', tone: 'warn' },
  TRIALACTIVE: { label: 'Trial', tone: 'warn' },
  PENDING: { label: 'Pending', tone: 'neutral' },
  PENDINGACTIVATION: { label: 'Pending', tone: 'neutral' },
  PENDINGCHARGE: { label: 'Processing', tone: 'warn' },
  CANCELED: { label: 'Canceled', tone: 'neutral' },
  CANCELLED: { label: 'Canceled', tone: 'neutral' },
  ENDED: { label: 'Ended', tone: 'neutral' },
  COMPLETED: { label: 'Ended', tone: 'neutral' },
  TRIALEXPIRED: { label: 'Trial expired', tone: 'danger' },
  ERRORED: { label: 'Errored', tone: 'danger' },
  SUSPENDED: { label: 'Suspended', tone: 'danger' },
  PAUSED: { label: 'Paused', tone: 'warn' },
  SUPERSEDED: { label: 'Replaced', tone: 'neutral' },
}

const ENUM_TO_KEY: Record<number, string> = {
  [SubscriptionStatus.PENDING]: 'PENDING',
  [SubscriptionStatus.TRIALING]: 'TRIALING',
  [SubscriptionStatus.ACTIVE]: 'ACTIVE',
  [SubscriptionStatus.CANCELED]: 'CANCELED',
  [SubscriptionStatus.ENDED]: 'ENDED',
  [SubscriptionStatus.TRIAL_EXPIRED]: 'TRIALEXPIRED',
  [SubscriptionStatus.ERRORED]: 'ERRORED',
}

/**
 * Badge for a subscription status. Accepts either the numeric proto enum
 * (SubscriptionSummary.status) or the Rust debug string
 * (SubscriptionDetails.status) — these are distinct wire shapes.
 */
export const subStatusBadge = (status: SubscriptionStatus | string): StatusBadge => {
  const key =
    typeof status === 'number'
      ? ENUM_TO_KEY[status]
      : status.replace(/[^a-zA-Z]/g, '').toUpperCase()
  return (
    BADGE_BY_KEY[key] ?? {
      label: typeof status === 'string' && status ? status : 'Unknown',
      tone: 'neutral',
    }
  )
}

/** Short label for a payment method, e.g. "Visa ···· 4242" or "Bank ···· 6789". */
export const pmLabel = (pm: CustomerPaymentMethod): string => {
  if (pm.paymentMethodType === PmType.CARD) {
    const brand = pm.cardBrand ? pm.cardBrand[0].toUpperCase() + pm.cardBrand.slice(1) : 'Card'
    return `${brand} ···· ${pm.cardLast4 ?? '••••'}`
  }
  return `Bank ···· ${pm.accountNumberHint ?? '••••'}`
}

export const pmSubLabel = (pm: CustomerPaymentMethod): string => {
  if (pm.paymentMethodType === PmType.CARD && pm.cardExpMonth && pm.cardExpYear) {
    return `Expires ${String(pm.cardExpMonth).padStart(2, '0')} / ${String(pm.cardExpYear).slice(-2)}`
  }
  return 'Bank account'
}

export const clampPct = (value: number, total: number) => {
  if (!total || total <= 0) return 0
  return Math.max(0, Math.min(100, Math.round((value / total) * 100)))
}
