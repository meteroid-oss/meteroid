import {
  type MetricUnit,
  formatPerUnit,
  rawQuantity,
  unitNoun,
} from '@/features/billing/metrics/units'
import { percentOf, unitIsCurrency } from '@/features/plans/charges/kinds'
import {
  type PriceDisplay,
  formatAmount,
  priceDisplay,
} from '@/features/plans/pricingpage/cardPrice'
import {
  type CardLevel,
  type CardLevels,
  CardExtra_Kind,
  type PricingCard,
} from '@/rpc/api/pricingpage/v1/pricingpage_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

// A card offering a usage-levels charge: the visitor moves between its levels, and a "More"
// stop past the largest when the ladder sells by contacting sales. "€479 / month · Up to €125K
// managed revenue per month · 0.5% overage on managed revenue above your plan limit".

const PER: Record<BillingPeriod, string> = {
  [BillingPeriod.MONTHLY]: 'month',
  [BillingPeriod.QUARTERLY]: 'quarter',
  [BillingPeriod.SEMIANNUAL]: '6 months',
  [BillingPeriod.ANNUAL]: 'year',
}

/** A stop of a card's levels: a level by its index, or "More" past the largest. */
export interface LevelStop {
  /** Short, under the slider: "€125K", "50K". */
  label: string
  /** Index in `CardLevels.levels`; none for "More". */
  level?: number
}

/** The levels' metric unit as the plan editor reads it. */
const metricUnit = (levels: CardLevels): MetricUnit => ({
  singular: levels.unitSingular ?? null,
  plural: levels.unitPlural ?? null,
  conversionFactor: levels.unitConversionFactor ?? null,
})

/** The unit named for display: its own names, else the metric's ("API calls"). */
const shownUnit = (levels: CardLevels): MetricUnit => {
  const unit = metricUnit(levels)
  if (unit.singular || unit.plural || !levels.metricName) return unit
  const name = lowerFirst(levels.metricName)
  return { ...unit, singular: name, plural: name }
}

/** Whether the levels count money in `currency`, by the plan editor's strict rule: their usage
 * then reads as an amount, their overage as a share of it. */
export const levelsCountMoney = (levels: CardLevels, currency: string): boolean =>
  unitIsCurrency(metricUnit(levels), currency)

/** "Managed revenue" → "managed revenue"; an acronym ("MRR", "API calls") keeps its case. */
const lowerFirst = (name: string) =>
  name.length > 1 && name[1] === name[1].toLowerCase()
    ? name[0].toLowerCase() + name.slice(1)
    : name

const compact = (value: number, locale?: string, currency?: string): string => {
  if (currency) {
    try {
      return new Intl.NumberFormat(locale, {
        style: 'currency',
        currency,
        currencyDisplay: 'narrowSymbol',
        notation: 'compact',
        minimumFractionDigits: 0,
        maximumFractionDigits: 1,
      }).format(value)
    } catch {
      // A credit unit code is no ISO currency: read as a count.
    }
  }
  return new Intl.NumberFormat(locale, { notation: 'compact', maximumFractionDigits: 1 }).format(
    value
  )
}

/** The usage a level includes in the metric's raw units. */
const rawIncluded = (levels: CardLevels, level: CardLevel): number =>
  rawQuantity(level.included, metricUnit(levels)) ?? Number(level.included)

/** "€125K managed revenue", "50K events": what a level includes. */
const includedLabel = (
  levels: CardLevels,
  level: CardLevel,
  currency: string,
  locale?: string
): string => {
  const raw = rawIncluded(levels, level)
  if (levelsCountMoney(levels, currency)) {
    const amount = compact(raw, locale, currency)
    return levels.metricName ? `${amount} ${lowerFirst(levels.metricName)}` : amount
  }
  return `${compact(raw, locale)} ${unitNoun(raw, shownUnit(levels))}`
}

/** The stops a visitor moves between: each level, smallest first, then "More" when offered. */
export const levelStops = (levels: CardLevels, currency: string, locale?: string): LevelStop[] => {
  const money = levelsCountMoney(levels, currency)
  const stops: LevelStop[] = levels.levels.map((level, index) => ({
    label: compact(rawIncluded(levels, level), locale, money ? currency : undefined),
    level: index,
  }))
  return levels.more && stops.length > 0 ? [...stops, { label: 'More' }] : stops
}

/** "0.5% overage on managed revenue above your plan limit", "$0.002 per event above your plan
 * limit"; none without an overage to pay. */
export const overageLabel = (
  levels: CardLevels,
  level: CardLevel,
  currency: string,
  locale?: string
): string | undefined => {
  const rate = level.overageRate
  if (!rate || !(Number(rate) > 0)) return undefined
  if (levelsCountMoney(levels, currency)) {
    const what = levels.metricName ? lowerFirst(levels.metricName) : 'usage'
    return `${percentOf(rate)} overage on ${what} above your plan limit`
  }
  const price = formatAmount(rate, currency, locale)
  return `${price} ${formatPerUnit(shownUnit(levels))} above your plan limit`
}

/** The card as it reads with `level`: its amount and base price that level's, the levels' own
 * extra left to the overage line. A saving reads at the lowest level only, the one the server
 * compared. */
export const cardAtLevel = <
  C extends Pick<PricingCard, 'free' | 'price' | 'hidePrice' | 'optionNote'>,
>(
  card: C,
  level: CardLevel
): C => {
  const price = card.price
  if (!price) return card
  return {
    ...card,
    price: {
      ...price,
      amount: level.amount,
      monthlyEquivalent: level.monthlyEquivalent,
      savingPercent: level.amount === price.amount ? price.savingPercent : undefined,
      fromLowest: false,
      extras: price.extras
        .filter(extra => !extra.levels)
        .map(extra =>
          extra.kind === CardExtra_Kind.BASE ? { ...extra, amount: level.amount } : extra
        ),
    },
  }
}

export type LevelCta =
  | { kind: 'signup'; params: URLSearchParams }
  /** "More": sales, the card's own or the ladder's sales-led plan (`planName`). */
  | { kind: 'contact'; url?: string; planName?: string }

/** What the card shows at one of its stops. */
export interface LevelReading {
  /** The card's price at the stop; "Custom pricing" at "More". */
  price: PriceDisplay
  /** "Up to €125K managed revenue per month"; "More than €500K …" at "More". */
  limit: string
  /** At a level with an overage to pay. */
  overage?: string
  cta: LevelCta
}

/** What the card's button passes on to sign-up: the plan, the currency and the option shown
 * (when the plan sells it, else its default applies), and at a level the usage it commits to,
 * `capacity`: the level's included usage, the committed capacity of the levels charge. */
export const signupParams = (
  card: Pick<PricingCard, 'planId' | 'options'>,
  page: { currency: string; option?: string },
  level?: CardLevel
): URLSearchParams => {
  const params = new URLSearchParams({ plan: card.planId, currency: page.currency })
  if (page.option && card.options.includes(page.option)) params.set('option', page.option)
  if (level) params.set('capacity', level.included.toString())
  return params
}

/** The card at `stop` (an index in `levelStops`), in the page's currency; none when it offers no
 * levels or the stop is not one of them. */
export const levelReading = (
  card: Pick<
    PricingCard,
    'planId' | 'options' | 'free' | 'price' | 'hidePrice' | 'optionNote' | 'salesLed' | 'contactUrl'
  >,
  stop: number,
  page: { currency: string; option?: string },
  locale?: string
): LevelReading | undefined => {
  const levels = card.price?.levels
  if (!levels || levels.levels.length === 0) return undefined
  const { currency } = page
  const period = card.price?.period
  const per = period === undefined ? '' : ` per ${PER[period]}`
  const level = levels.levels[stop]
  if (!level) {
    if (!levels.more || stop !== levels.levels.length) return undefined
    const largest = levels.levels[levels.levels.length - 1]
    return {
      price: { kind: 'custom' },
      limit: `More than ${includedLabel(levels, largest, currency, locale)}${per}`,
      cta: { kind: 'contact', url: levels.moreContactUrl, planName: levels.morePlanName },
    }
  }
  return {
    price: priceDisplay(cardAtLevel(card, level), currency, locale),
    limit: `Up to ${includedLabel(levels, level, currency, locale)}${per}`,
    overage: overageLabel(levels, level, currency, locale),
    // A sales-led card's button contacts sales at every level.
    cta: card.salesLed
      ? { kind: 'contact', url: card.contactUrl }
      : { kind: 'signup', params: signupParams(card, page, level) },
  }
}
