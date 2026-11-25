import { create } from '@bufbuild/protobuf'

import { currencyPrecision } from '@/lib/data/currency-precision'
import {
  CardExtraSchema,
  CardExtra_Kind,
  type CardExtra,
  type CardPrice,
  type PricingCard,
} from '@/rpc/api/pricingpage/v1/pricingpage_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

const SHORT: Record<BillingPeriod, string> = {
  [BillingPeriod.MONTHLY]: 'mo',
  [BillingPeriod.QUARTERLY]: 'qtr',
  [BillingPeriod.SEMIANNUAL]: '6 mo',
  [BillingPeriod.ANNUAL]: 'yr',
}

// A card's price reads per month, as the page compares plans: "$41 / month".
const PER: Record<BillingPeriod, string> = {
  [BillingPeriod.MONTHLY]: 'month',
  [BillingPeriod.QUARTERLY]: 'quarter',
  [BillingPeriod.SEMIANNUAL]: '6 months',
  [BillingPeriod.ANNUAL]: 'year',
}

const BILLED: Record<BillingPeriod, string> = {
  [BillingPeriod.MONTHLY]: 'monthly',
  [BillingPeriod.QUARTERLY]: 'quarterly',
  [BillingPeriod.SEMIANNUAL]: 'every 6 months',
  [BillingPeriod.ANNUAL]: 'yearly',
}

const NAMES: Record<BillingPeriod, string> = {
  [BillingPeriod.MONTHLY]: 'Monthly',
  [BillingPeriod.QUARTERLY]: 'Quarterly',
  [BillingPeriod.SEMIANNUAL]: 'Every 6 months',
  [BillingPeriod.ANNUAL]: 'Yearly',
}

export const cadenceName = (cadence: BillingPeriod) => NAMES[cadence] ?? ''

/** `149` → `$149`, `72.5` → `$72.50`, `0.002` → `$0.002`: whole amounts drop their decimals. */
export const formatAmount = (amount: string, currency: string, locale?: string): string => {
  const value = Number(amount)
  if (!amount.trim() || !Number.isFinite(value)) return amount
  const decimals = (amount.split('.')[1] ?? '').replace(/0+$/, '').length
  const min = Number.isInteger(value) ? 0 : currencyPrecision(currency)
  const max = Math.min(Math.max(min, decimals), 8)
  try {
    return new Intl.NumberFormat(locale, {
      style: 'currency',
      currency,
      currencyDisplay: 'narrowSymbol',
      minimumFractionDigits: min,
      maximumFractionDigits: max,
    }).format(value)
  } catch {
    // A credit unit code is no ISO currency.
    const number = new Intl.NumberFormat(locale, {
      minimumFractionDigits: min,
      maximumFractionDigits: max,
    }).format(value)
    return `${number} ${currency}`
  }
}

/** "seat" → "seats"; a unit already plural stays. */
const plural = (unit: string) => (/s$/i.test(unit) ? unit : `${unit}s`)

const count = (value: bigint | number, locale?: string) =>
  new Intl.NumberFormat(locale).format(value)

const unitOf = (extra: CardExtra) =>
  extra.unit || (extra.kind === CardExtra_Kind.PER_SEAT ? 'seat' : 'unit')

/** `/ seat`, `/ 10 units`: what an item's amount is for. */
const perUnit = (extra: CardExtra, locale?: string) =>
  extra.blockSize !== undefined && extra.blockSize > 1n
    ? `/ ${count(extra.blockSize, locale)} ${plural(unitOf(extra))}`
    : `/ ${unitOf(extra)}`

/** `3 seats included`, `1,000,000 events included`: units free before the price applies. */
const includedLabel = (extra: CardExtra, locale?: string) => {
  const included = extra.included
  if (included === undefined || included <= 0n) return undefined
  const unit = included === 1n ? unitOf(extra) : plural(unitOf(extra))
  return `${count(included, locale)} ${unit} included`
}

/** What an item reads: its figure, what it is, and the units it includes. */
interface ExtraParts {
  /** `$99 / mo`, `from $8 / seat`, `$500`. */
  figure?: string
  /** `base price`, `Support hours`, `once`, `usage`. */
  label?: string
  included?: string
}

const extraParts = (
  extra: CardExtra,
  currency: string,
  locale?: string,
  period?: BillingPeriod
): ExtraParts => {
  const money = extra.amount ? formatAmount(extra.amount, currency, locale) : undefined
  switch (extra.kind) {
    case CardExtra_Kind.BASE:
      return {
        figure: money && (period !== undefined ? `${money} / ${SHORT[period]}` : money),
        label: 'base price',
      }
    case CardExtra_Kind.PER_SEAT:
    case CardExtra_Kind.USAGE:
      if (!money) {
        return { label: extra.kind === CardExtra_Kind.USAGE ? 'usage' : extra.name || 'seats' }
      }
      return {
        figure: `${extra.fromLowest ? 'from ' : ''}${money} ${perUnit(extra, locale)}`,
        included: includedLabel(extra, locale),
      }
    case CardExtra_Kind.ONE_TIME:
      return money ? { figure: money, label: 'once' } : { label: extra.name || 'one-time fee' }
    case CardExtra_Kind.RECURRING:
      // Billed on its own period, never folded into the card's amount.
      return money && extra.period !== undefined
        ? { figure: `${money} / ${SHORT[extra.period]}`, label: extra.name }
        : { label: extra.name }
    default:
      return { label: extra.name }
  }
}

const joined = ({ figure, label, included }: ExtraParts) =>
  `+ ${[figure, label].filter(Boolean).join(' ')}${included ? ` (${included})` : ''}`

/** `+ $12 / seat`, `+ from $8 / seat`, `+ $0.002 / event`, `+ usage`, `+ $500 once`,
 * `+ $99 / mo base price` (on the card's `period`). */
export const extraLabel = (
  extra: CardExtra,
  currency: string,
  locale?: string,
  period?: BillingPeriod
): string => joined(extraParts(extra, currency, locale, period))

/** Usage priced in a way one rate can't say (rates by dimension, a rate card): read by name. */
const unpricedUsage = (extra: CardExtra) => extra.kind === CardExtra_Kind.USAGE && !extra.amount

/** `+ LLM tokens`, `+ usage` when it has no name. */
export const usageLabel = (extra: Pick<CardExtra, 'name'>): string =>
  extra.name ? `+ ${extra.name}` : '+ usage'

/** An item under the card's price, read as one phrase (`text`) or in parts. */
export interface CardLine {
  /** `+ $0.0004 / event (1,000,000 events included)`. */
  text: string
  figure?: string
  label?: string
  /** Short, the unit left to the figure: `1M included`. */
  included?: string
}

const cardLine = (
  extra: CardExtra,
  price: Pick<CardPrice, 'extras' | 'levels' | 'period'>,
  currency: string,
  locale?: string
): CardLine => {
  if (unpricedUsage(extra)) {
    const text = usageLabel(extra)
    return { text, label: text.replace(/^\+ /, '') }
  }
  const parts = extraParts(extra, currency, locale, price.period)
  return {
    text: joined(parts),
    figure: parts.figure,
    label: parts.label,
    included:
      parts.included && extra.included !== undefined
        ? `${shortCount(extra.included, locale)} included`
        : undefined,
  }
}

/** `1M`, `250K`, `12.5K` when that is exact, else in full: `1,234,567`. */
const shortCount = (value: bigint, locale?: string) => {
  const digits = value.toString().replace(/0+$/, '')
  return value >= 10_000n && digits.length <= 3
    ? new Intl.NumberFormat(locale, { notation: 'compact', maximumFractionDigits: 2 }).format(value)
    : count(value, locale)
}

export type PriceDisplay =
  | { kind: 'free' }
  /** A sales-led plan whose price is not shown: "Custom pricing". */
  | { kind: 'custom' }
  | { kind: 'unavailable'; label: string }
  | {
      kind: 'priced'
      amount: string
      per: string
      /** The amount is the lowest rate of its tiers: "from $8". */
      from?: boolean
      /** Under the amount: "3 seats included". */
      included?: string
      /** `$1,490 billed yearly` under its monthly equivalent, for periods longer than a month. */
      billed?: string
      saving?: string
      /** The card's other items in its order, the base price among them when another leads. */
      extras: CardLine[]
      /** The plan lacks the option shown and reads in its default: "Billed monthly". */
      note?: string
      /** The item the server says leads, when it is not the base price. */
      lead?: CardExtra
    }

/** What a card's price block shows, in the page's currency: the item leading (the server marks
 * the first of its order that can), the others below it in that order. */
export const priceDisplay = (
  card: Pick<PricingCard, 'free' | 'price' | 'hidePrice' | 'optionNote' | 'salesLed'>,
  currency: string,
  locale?: string
): PriceDisplay => {
  if (card.hidePrice) return { kind: 'custom' }
  if (card.free) return { kind: 'free' }
  const price = card.price
  // A plan lacking the option shown reads in its default: none only when not sold in the currency,
  // which leaves it off the page unless sales sell it.
  if (!price) {
    return card.salesLed
      ? { kind: 'custom' }
      : { kind: 'unavailable', label: `Not offered in ${currency}` }
  }
  const note = card.optionNote || undefined
  const saving = price.savingPercent ? `Save ${price.savingPercent}%` : undefined
  const perMonth =
    price.monthlyEquivalent && price.period !== undefined && price.period !== BillingPeriod.MONTHLY
      ? price.monthlyEquivalent
      : undefined
  const per = price.period === undefined ? '' : `/ ${PER[price.period]}`
  // Each phrase once: usage without a rate reads a single line however many charges it folds.
  const labels = (list: CardExtra[]) =>
    list
      .map(extra => cardLine(extra, price, currency, locale))
      .filter((line, index, lines) => lines.findIndex(l => l.text === line.text) === index)
  // A card from before the base price was an item reads it first, and when nothing marks a
  // lead, what it charges for leads when nothing fixed is due: seats first, then usage.
  const listed = price.extras.some(extra => extra.kind === CardExtra_Kind.BASE)
  const items =
    listed || Number(price.amount) === 0
      ? price.extras
      : [
          create(CardExtraSchema, { kind: CardExtra_Kind.BASE, amount: price.amount }),
          ...price.extras,
        ]
  const marked = items.find(e => e.lead)
  const lead =
    marked ??
    (!listed && Number(price.amount) === 0
      ? (items.find(e => e.kind === CardExtra_Kind.PER_SEAT && e.amount) ??
        items.find(e => e.kind === CardExtra_Kind.USAGE && e.amount))
      : undefined)
  if (lead?.amount && lead.kind !== CardExtra_Kind.BASE) {
    return {
      kind: 'priced',
      amount: formatAmount(lead.amount, currency, locale),
      per:
        lead.kind === CardExtra_Kind.PER_SEAT
          ? `${perUnit(lead, locale)} ${per}`.trim()
          : perUnit(lead, locale),
      from: lead.fromLowest || undefined,
      included: includedLabel(lead, locale),
      saving,
      // The base price still reads, where the order puts it.
      extras: labels(items.filter(extra => extra !== lead)),
      note,
      lead,
    }
  }
  const extras = labels(items.filter(extra => extra.kind !== CardExtra_Kind.BASE))
  if (perMonth && price.period !== undefined) {
    return {
      kind: 'priced',
      amount: formatAmount(perMonth, currency, locale),
      per: `/ ${PER[BillingPeriod.MONTHLY]}`,
      billed: `${formatAmount(price.amount, currency, locale)} billed ${BILLED[price.period]}`,
      saving,
      extras,
      note,
    }
  }
  return {
    kind: 'priced',
    amount: formatAmount(price.amount, currency, locale),
    // A usage-only plan has no period of its own.
    per,
    saving,
    extras,
    note,
  }
}

// A card read in its default option (`optionNote`) saves nothing on the option shown.
const savingOf = (card: Pick<PricingCard, 'price' | 'optionNote'>) =>
  card.optionNote ? undefined : card.price?.savingPercent

/** The best saving among the cards, in whole percent; 0 when none saves. */
export const bestSaving = (cards: Pick<PricingCard, 'price' | 'optionNote'>[]): number =>
  cards.reduce((best, card) => Math.max(best, savingOf(card) ?? 0), 0)

/** `Yearly · save up to 17%`; other periods by name. */
export const cadenceOptionLabel = (cadence: BillingPeriod, saving = 0): string =>
  cadence === BillingPeriod.ANNUAL && saving > 0
    ? `${NAMES[cadence]} · save up to ${saving}%`
    : cadenceName(cadence)

/** "2 months free" when every card saves the same whole months on a yearly option, else
 * "save up to 17%"; none when no card saves. */
export const optionSavingLabel = (
  cards: Pick<PricingCard, 'price' | 'optionNote'>[]
): string | undefined => {
  const saving = cards.flatMap(card => {
    const percent = savingOf(card)
    return percent ? [{ percent, period: card.price?.period }] : []
  })
  if (saving.length === 0) return undefined
  const best = Math.max(...saving.map(s => s.percent))
  const same = saving.every(s => s.percent === best)
  const months = Math.round((best * 12) / 100)
  // Saving percents are whole: 2 months of 12 reads 17%.
  if (
    same &&
    months >= 1 &&
    saving.every(s => s.period === BillingPeriod.ANNUAL) &&
    Math.abs((months * 100) / 12 - best) < 1
  ) {
    return `${months} ${months === 1 ? 'month' : 'months'} free`
  }
  return same ? `save ${best}%` : `save up to ${best}%`
}

export const ctaLabelOf = (
  card: Pick<PricingCard, 'ctaLabel' | 'trialDays' | 'salesLed'>
): string =>
  card.ctaLabel?.trim() ||
  (card.salesLed
    ? 'Contact sales'
    : card.trialDays
      ? `Start a ${card.trialDays}-day trial`
      : 'Get started')
