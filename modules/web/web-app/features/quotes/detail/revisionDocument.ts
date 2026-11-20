import { creditExpiryLabel } from '@/features/productCatalog/coupons/discountLabel'
import { formatCurrencyNoRounding } from '@/utils/numbers'

type Json = Record<string, unknown>

export interface RevisionLine {
  name: string
  description?: string
  period: string
  price: string
  listPrice?: string
  discountPercent?: string
}

export interface RevisionCoupon {
  code: string
  description?: string
  discount: string
  /** Undefined for a promotional credit, which has no duration. */
  duration?: string
}

export interface RevisionSummary {
  quoteNumber?: string
  currency?: string
  customer?: string
  seller?: string
  startDate?: string
  endDate?: string
  expiresAt?: string
  trialDays?: number
  netTerms?: number
  lines: RevisionLine[]
  coupons: RevisionCoupon[]
  sections: { title: string; body: string }[]
  overview?: string
  terms?: string
  recipients: string[]
  countersigners: string[]
  consentText?: string
}

const str = (v: unknown) =>
  typeof v === 'string' ? v : typeof v === 'number' ? String(v) : undefined
const obj = (v: unknown): Json => (v && typeof v === 'object' ? (v as Json) : {})

const PERIODS: Record<string, string> = {
  OneTime: 'One time',
  Monthly: 'Monthly',
  Quarterly: 'Quarterly',
  Semiannual: 'Semiannual',
  Annual: 'Yearly',
}

const formatFee = (fee: unknown, currency: string): string => {
  const [variant, raw] = Object.entries(obj(fee))[0] ?? []
  if (!variant) return '—'
  const value = obj(raw)
  const money = (v: unknown) => {
    const s = str(v)
    return s === undefined ? '—' : formatCurrencyNoRounding(s, currency)
  }
  const quantity = Number(value.quantity ?? 1)
  const times = quantity > 1 ? `${quantity} × ` : ''
  switch (variant) {
    case 'Rate':
      return money(value.rate)
    case 'OneTime':
    case 'Recurring':
      return `${times}${money(value.rate)}`
    case 'Credit':
      return `${times}${money(value.rate)} (${str(value.credit_amount) ?? '?'} credit)`
    case 'Capacity':
      return `${money(value.rate)}, ${str(value.included) ?? 0} included, then ${money(value.overage_rate)}/unit`
    case 'Slot':
      return `${money(value.unit_rate)} per ${str(value.unit) ?? 'slot'}`
    case 'Usage': {
      const [model, params] = Object.entries(obj(value.model))[0] ?? []
      const rate = str(obj(params).rate)
      return rate !== undefined
        ? `Usage (${model}): ${money(rate)}`
        : `Usage (${model ?? 'metered'})`
    }
    default:
      return variant
  }
}

const formatCouponDiscount = (variant: string | undefined, raw: unknown): string => {
  const value = obj(raw)
  const money = (amount: unknown, currency: unknown) => {
    const a = str(amount)
    const c = str(currency)
    return a !== undefined && c !== undefined ? formatCurrencyNoRounding(a, c) : '—'
  }
  switch (variant) {
    case 'Percentage':
      return `${str(raw) ?? '—'}% off`
    case 'Fixed':
      return `${money(value.amount, value.currency)} off`
    case 'Credit': {
      const days = typeof value.expires_in_days === 'number' ? value.expires_in_days : undefined
      return [
        `${money(value.amount, value.currency)} credit`,
        days !== undefined && creditExpiryLabel(days).toLowerCase(),
        value.scope === 'SUBSCRIPTION' && 'subscription only',
      ]
        .filter(Boolean)
        .join(', ')
    }
    default:
      return 'Discount'
  }
}

const parseCoupon = (coupon: Json): RevisionCoupon => {
  const [variant, raw] = Object.entries(obj(coupon.discount))[0] ?? []
  const periods = typeof coupon.recurring_value === 'number' ? coupon.recurring_value : undefined
  return {
    code: str(coupon.code) ?? '',
    description: str(coupon.description),
    discount: formatCouponDiscount(variant, raw),
    duration:
      variant === 'Credit'
        ? undefined
        : periods === undefined
          ? 'Forever'
          : periods === 1
            ? 'Once'
            : `First ${periods} billing periods`,
  }
}

export const parseRevisionDocument = (document: string): RevisionSummary | undefined => {
  let doc: Json
  try {
    doc = obj(JSON.parse(document))
  } catch {
    return undefined
  }
  const currency = str(doc.currency) ?? 'USD'
  const list = (v: unknown) => (Array.isArray(v) ? v.map(obj) : [])

  return {
    quoteNumber: str(doc.quote_number),
    currency,
    customer: str(obj(doc.customer).name),
    seller: str(obj(doc.seller).name),
    startDate: str(doc.billing_start_date),
    endDate: str(doc.billing_end_date),
    expiresAt: str(doc.expires_at),
    trialDays: typeof doc.trial_duration_days === 'number' ? doc.trial_duration_days : undefined,
    netTerms: typeof doc.net_terms === 'number' ? doc.net_terms : undefined,
    lines: list(doc.lines).map(line => {
      const quantity =
        typeof line.quantity === 'number' && line.quantity > 1 ? ` (x${line.quantity})` : ''
      return {
        name: `${str(line.name) ?? ''}${quantity}`,
        description: str(line.description),
        period: PERIODS[str(line.period) ?? ''] ?? str(line.period) ?? '',
        price: formatFee(line.fee, currency),
        listPrice: line.list_fee ? formatFee(line.list_fee, currency) : undefined,
        discountPercent: str(line.discount_percent),
      }
    }),
    coupons: list(doc.coupons).map(parseCoupon),
    sections: list(doc.sections).map(s => ({ title: str(s.title) ?? '', body: str(s.body) ?? '' })),
    overview: str(doc.overview),
    terms: str(doc.terms_and_services),
    recipients: list(doc.recipients).map(r => `${str(r.name) ?? ''} <${str(r.email) ?? ''}>`),
    countersigners: list(doc.countersigners).map(
      c => `${str(c.name) ?? ''} <${str(c.email) ?? ''}>${c.automatic ? ' (automatic)' : ''}`
    ),
    consentText: str(doc.consent_text),
  }
}
