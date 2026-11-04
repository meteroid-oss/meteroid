import { formatPackLineCredit } from '@/features/subscriptions/planChangePackLines'
import { currencyPrecision } from '@/lib/data/currency-precision'
import {
  CustomerPaymentMethod,
  CustomerPaymentMethod_PaymentMethodTypeEnum as PmType,
} from '@/rpc/api/customers/v1/models_pb'
import { InvoicePaymentStatus, InvoiceStatus } from '@/rpc/api/invoices/v1/models_pb'
import { FeeType } from '@/rpc/api/prices/v1/models_pb'
import {
  PlanChangePackLine_Kind,
  SubscriptionFeeBillingPeriod,
  SubscriptionStatus,
} from '@/rpc/api/subscriptions/v1/models_pb'

import type {
  PlanChangePackLine,
  SubscriptionFee_FeeCreditUnit,
} from '@/rpc/api/subscriptions/v1/models_pb'
import type { InvoiceSummary } from '@/rpc/portal/customer/v1/models_pb'
import type { ComponentFee } from '@/rpc/portal/subscription/v1/subscription_pb'

/** Format an integer amount of minor units in its currency, at the currency's own precision. */
export const money = (cents: bigint | number, currency: string, opts?: { compact?: boolean }) => {
  const precision = currencyPrecision(currency)
  const value = Number(cents) / Math.pow(10, precision)
  try {
    return new Intl.NumberFormat('en-US', {
      style: 'currency',
      currency: currency || 'USD',
      minimumFractionDigits: opts?.compact && Number.isInteger(value) ? 0 : precision,
      maximumFractionDigits: precision,
    }).format(value)
  } catch {
    return `${currency} ${value.toFixed(precision)}`
  }
}

/** Format a decimal-string amount (e.g. a component rate) in its currency. */
export const moneyStr = (amount: string, currency: string) => {
  const n = Number(amount)
  if (Number.isNaN(n)) return amount
  return money(Math.round(n * Math.pow(10, currencyPrecision(currency))), currency)
}

const decimalPlaces = (value: string): number => {
  const i = value.indexOf('.')
  return i === -1 ? 0 : value.length - i - 1
}

/**
 * Format a decimal-string conversion rate (e.g. a credit unit's price per credit) in its
 * currency, keeping every digit the string carries instead of rounding to cents like `moneyStr`
 * — a $0.001 rate must show as $0.001, not $0.00.
 */
export const rateStr = (amount: string, currency: string): string => {
  const n = Number(amount)
  if (Number.isNaN(n)) return amount
  const digits = Math.max(2, decimalPlaces(amount))
  try {
    return new Intl.NumberFormat('en-US', {
      style: 'currency',
      currency: currency || 'USD',
      minimumFractionDigits: 2,
      maximumFractionDigits: digits,
    }).format(n)
  } catch {
    return `${currency} ${n.toFixed(digits)}`
  }
}

const DATE_FMT = new Intl.DateTimeFormat('en-US', {
  month: 'short',
  day: 'numeric',
  year: 'numeric',
})

const DATE_ONLY_FMT = new Intl.DateTimeFormat('en-US', {
  month: 'short',
  day: 'numeric',
  year: 'numeric',
  timeZone: 'UTC',
})

const DATE_ONLY = /^\d{4}-\d{2}-\d{2}$/

/** Format an ISO date/datetime string as "Jul 1, 2026". Empty when missing. */
export const date = (iso?: string) => {
  if (!iso) return ''
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return iso
  // A calendar date parses as UTC midnight: printing it in local time would shift it a day west of UTC.
  return DATE_ONLY.test(iso) ? DATE_ONLY_FMT.format(d) : DATE_FMT.format(d)
}

const DATE_TIME_FMT = new Intl.DateTimeFormat('en-US', {
  month: 'short',
  day: 'numeric',
  year: 'numeric',
  hour: 'numeric',
  minute: 'numeric',
})

/** Format an ISO date/datetime string as "Jul 1, 2026, 9:27 PM". Empty when missing. */
export const dateTime = (iso?: string) => {
  if (!iso) return ''
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return iso
  return DATE_TIME_FMT.format(d)
}

const PERIOD_SUFFIX: Record<SubscriptionFeeBillingPeriod, string> = {
  [SubscriptionFeeBillingPeriod.ONE_TIME]: 'one-time',
  [SubscriptionFeeBillingPeriod.MONTHLY]: '/mo',
  [SubscriptionFeeBillingPeriod.QUARTERLY]: '/qtr',
  [SubscriptionFeeBillingPeriod.YEARLY]: '/yr',
  [SubscriptionFeeBillingPeriod.SEMIANNUAL]: '/6mo',
}

export const periodSuffix = (cadence: SubscriptionFeeBillingPeriod) => PERIOD_SUFFIX[cadence] ?? ''

const CADENCE_NOUN: Record<SubscriptionFeeBillingPeriod, string> = {
  [SubscriptionFeeBillingPeriod.ONE_TIME]: '',
  [SubscriptionFeeBillingPeriod.MONTHLY]: 'month',
  [SubscriptionFeeBillingPeriod.QUARTERLY]: 'quarter',
  [SubscriptionFeeBillingPeriod.SEMIANNUAL]: '6 months',
  [SubscriptionFeeBillingPeriod.YEARLY]: 'year',
}

const CADENCE_ADJECTIVE: Record<SubscriptionFeeBillingPeriod, string> = {
  [SubscriptionFeeBillingPeriod.ONE_TIME]: 'one-time',
  [SubscriptionFeeBillingPeriod.MONTHLY]: 'monthly',
  [SubscriptionFeeBillingPeriod.QUARTERLY]: 'quarterly',
  [SubscriptionFeeBillingPeriod.SEMIANNUAL]: 'semiannual',
  [SubscriptionFeeBillingPeriod.YEARLY]: 'annual',
}

/** "month", "quarter", "6 months", "year"; empty for a one-time charge. */
export const cadenceNoun = (cadence: SubscriptionFeeBillingPeriod) => CADENCE_NOUN[cadence] ?? ''

/** "/ month" after a price; "one-time" for a one-time charge. */
export const cadenceSuffix = (cadence: SubscriptionFeeBillingPeriod) => {
  const noun = cadenceNoun(cadence)
  return noun ? `/ ${noun}` : 'one-time'
}

/** "every month"; "once" for a one-time charge. */
export const everyCadence = (cadence: SubscriptionFeeBillingPeriod) => {
  const noun = cadenceNoun(cadence)
  return noun ? `every ${noun}` : 'once'
}

export const cadenceAdjective = (cadence: SubscriptionFeeBillingPeriod) =>
  CADENCE_ADJECTIVE[cadence] ?? ''

/** A decimal-string amount with thousands separators, e.g. "5000" -> "5,000". */
export const decimalStr = (amount: string): string => {
  const n = Number(amount)
  if (Number.isNaN(n)) return amount
  return new Intl.NumberFormat('en-US', { maximumFractionDigits: decimalPlaces(amount) }).format(n)
}

/**
 * Human pricing label for a headline component fee, e.g.:
 *   Rate     → "$290.00 /mo"
 *   Slot     → "$12.00 /seat /mo"
 *   Usage    → "Usage-based", or "Usage-based, in Halyard credits (1 Halyard credit = $0.01)"
 *   Capacity → "$0.10 /unit + base", with the same credit-unit note on the overage
 *   Credit   → "5,000 Halyard credits for $40.00 /mo" when the pack grants a credit unit
 */
export const feeLabel = (fee: ComponentFee | undefined, currency: string): string => {
  if (!fee) return '—'
  const suffix = periodSuffix(fee.cadence)
  const unit = fee.creditUnit
  // The unit's code is internal: without its label, it is just "credits".
  const singular = fee.creditUnitLabel?.name ?? 'credit'
  const plural = fee.creditUnitLabel?.pluralName ?? 'credits'
  const rateNote = unit?.rate ? ` (1 ${singular} = ${rateStr(unit.rate, currency)})` : ''
  switch (fee.feeType) {
    case FeeType.USAGE:
      return unit ? `Usage-based, in ${plural}${rateNote}` : 'Usage-based'
    case FeeType.SLOT: {
      const slotUnit = fee.unit ? ` /${fee.unit}` : ' /seat'
      return `${moneyStr(fee.amount, currency)}${slotUnit} ${suffix}`.trim()
    }
    case FeeType.ONE_TIME:
      return `${moneyStr(fee.amount, currency)} one-time`
    case FeeType.CREDIT:
      return unit && fee.creditAmount
        ? `${decimalStr(fee.creditAmount)} ${Number(fee.creditAmount) === 1 ? singular : plural} for ${moneyStr(fee.amount, currency)} ${suffix}`.trim()
        : `${moneyStr(fee.amount, currency)} ${suffix}`.trim()
    case FeeType.CAPACITY:
      return `${moneyStr(fee.amount, currency)} ${suffix}${unit ? `, overage in ${plural}${rateNote}` : ''}`.trim()
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
  ABORTED: { label: 'Aborted', tone: 'neutral' },
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
  [SubscriptionStatus.ABORTED]: 'ABORTED',
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

const COMPACT_FMT = new Intl.NumberFormat('en-US', {
  notation: 'compact',
  maximumFractionDigits: 2,
})

/** A quantity in compact notation, e.g. "8750000" -> "8.75M". */
export const compactNumber = (value: string | number): string => {
  const n = Number(value)
  return Number.isNaN(n) ? String(value) : COMPACT_FMT.format(n)
}

/** Map an invoice's payment status (or draft state) to a display badge. */
export const invoiceBadge = (invoice: {
  status: InvoiceStatus
  paymentStatus: InvoicePaymentStatus
  settledByCreditNotes?: boolean
}): StatusBadge => {
  if (invoice.status === InvoiceStatus.DRAFT) {
    return { label: 'Draft', tone: 'neutral' }
  }
  if (invoice.status === InvoiceStatus.CLOSED) {
    return { label: 'Closed', tone: 'neutral' }
  }
  switch (invoice.paymentStatus) {
    case InvoicePaymentStatus.PAID:
      return { label: invoice.settledByCreditNotes ? 'Credited' : 'Paid', tone: 'neutral' }
    case InvoicePaymentStatus.PARTIALLY_PAID:
      return { label: 'Partially paid', tone: 'warn' }
    case InvoicePaymentStatus.ERRORED:
      return { label: 'Errored', tone: 'danger' }
    case InvoicePaymentStatus.PROCESSING:
      return { label: 'Processing', tone: 'warn' }
    case InvoicePaymentStatus.UNPAID:
    default:
      return { label: 'Unpaid', tone: 'warn' }
  }
}

/** What the invoice charges the customer once credit is applied, and the credit sub-line if any. */
export const invoiceCharge = (invoice: InvoiceSummary) => {
  const charged = invoice.totalCents - invoice.appliedCreditsCents
  const creditLine =
    invoice.appliedCreditsCents > 0n
      ? `${money(invoice.totalCents, invoice.currency)} total · ${money(invoice.appliedCreditsCents, invoice.currency)} paid with credit`
      : undefined
  return { charged, creditLine }
}

export const clampPct = (value: number, total: number) => {
  if (!total || total <= 0) return 0
  return Math.max(0, Math.min(100, Math.round((value / total) * 100)))
}

// The portal never shows a unit code: the server sets the unit's names on each line.
const portalUnitNames = (unit: SubscriptionFee_FeeCreditUnit) => ({
  name: unit.name || 'credit',
  pluralName: unit.pluralName || 'credits',
  precision: unit.precision,
})

/** A plan-change credit pack line, in the customer's words. */
export const packLineView = (line: PlanChangePackLine, currency: string) => {
  const credit = () => formatPackLineCredit(line, currency, portalUnitNames)
  switch (line.kind) {
    case PlanChangePackLine_Kind.CHARGE:
      return {
        label: `Credit included now · ${credit()}`,
        value: `+${money(line.amountCents, currency)}`,
      }
    case PlanChangePackLine_Kind.CREDIT_REMOVED:
      return {
        label: line.promotional
          ? 'Unused bonus or included credit removed'
          : 'Unused credit removed',
        value: `−${credit()}`,
      }
    case PlanChangePackLine_Kind.FEE_CREDIT:
      return { label: 'Refund for unused credit pack', value: money(line.amountCents, currency) }
    default:
      return undefined
  }
}

/**
 * When the next invoice is paid, as the overview states it in its header and its Next payment
 * card: charged to the payment method on its charge date, else due on its due date.
 */
export const nextPayment = <M>(invoice: {
  chargeAutomatically: boolean
  chargeDate?: string
  paymentMethod?: M
  dueDate?: string
  invoiceDate: string
}): { charged: boolean; date: string; paymentMethod?: M } => {
  const paymentMethod = invoice.chargeAutomatically ? invoice.paymentMethod : undefined
  return paymentMethod
    ? { charged: true, date: invoice.chargeDate || invoice.invoiceDate, paymentMethod }
    : { charged: false, date: invoice.dueDate || invoice.invoiceDate }
}
