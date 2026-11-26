import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import type { Cadence } from '@/features/plans/charges/compile'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'

// A plan charge's price cell is (charge, option, currency): what a buyer of that option pays in
// that currency. Options are what a buyer picks; a price's cadence is only how it bills.

type Priced = Pick<Price, 'pricing' | 'cadence' | 'currency'>
type Linked = Pick<PriceComponent, 'prices' | 'optionPrices'>
type Charge = Pick<PriceComponent, 'id' | 'name' | 'prices' | 'optionPrices'>
/** `hidden`: sold when named (quotes, links, the API), off the pricing page and portal. */
export type OptionLike = { id: string; name: string; alias: string; hidden?: boolean }

/**
 * The prices of one cell: those `option` links in `currency` (capacity levels are several). A
 * charge linked to no option (a v1 version, made before options) has its prices in every one.
 */
export const optionCell = <C extends Linked>(
  component: C,
  optionId: string,
  currency: string
): C['prices'] => {
  if (component.optionPrices.length === 0) {
    return component.prices.filter(p => p.currency === currency)
  }
  const linked = new Set(
    component.optionPrices.filter(link => link.optionId === optionId).map(link => link.priceId)
  )
  return component.prices.filter(p => linked.has(p.id) && p.currency === currency)
}

const sameIds = (a: Pick<Price, 'id'>[], b: Pick<Price, 'id'>[]) =>
  a.length === b.length && a.every(p => b.some(q => q.id === p.id))

/**
 * Every option links the same prices in `currency`: the row shows one cell for them all. Not
 * priced there yet, it still does when every option shares the charge's prices elsewhere.
 */
export function sameInEveryOption(
  component: Linked,
  options: Pick<OptionLike, 'id'>[],
  currency: string
): boolean {
  if (options.length < 2) return false
  const cells = options.map(option => optionCell(component, option.id, currency))
  if (!cells.every(cell => sameIds(cell, cells[0]))) return false
  if (cells[0].length > 0) return true
  const links = options.map(option =>
    component.optionPrices.filter(l => l.optionId === option.id).map(l => ({ id: l.priceId }))
  )
  return links[0].length > 0 && links.every(linked => sameIds(linked, links[0]))
}

/**
 * The priced charges, usage and one-time included, with no price for `option` in `currency`:
 * the option is not sold there until each has one. A charge with no price at all is included.
 */
export const unpricedIn = <C extends Charge>(components: C[], optionId: string, currency: string): C[] =>
  components.filter(c => c.prices.length > 0 && optionCell(c, optionId, currency).length === 0)

/** The options sold in `currency`: every priced charge has a cell for them there. */
export const soldOptions = <O extends Pick<OptionLike, 'id'>>(
  options: O[],
  components: Charge[],
  currency: string
): O[] => options.filter(option => unpricedIn(components, option.id, currency).length === 0)

/** A currency tab's warning dot: an option lacks a price there. */
export const tabWarns = (components: Charge[], options: Pick<OptionLike, 'id'>[], currency: string) =>
  soldOptions(options, components, currency).length < options.length

/** The version's currency first, then the others alphabetically; duplicates dropped. */
export function orderCurrencies(currencies: string[], defaultCurrency: string): string[] {
  const unique = [...new Set(currencies)]
  return unique.sort(
    (a, b) => Number(b === defaultCurrency) - Number(a === defaultCurrency) || a.localeCompare(b)
  )
}

/** The currencies this version prices in: at least one of its prices is in each. */
export const pricedCurrencies = (components: Pick<PriceComponent, 'prices'>[]): string[] => [
  ...new Set(components.flatMap(c => c.prices.map(p => p.currency)).filter(Boolean)),
]

/** "USD · Annual" where a plan is sold; just "USD" for a plan with one option. */
export const pricedOptionLabel = (
  priced: { currency: string; optionId: string },
  options: OptionLike[]
) => {
  const option = options.find(o => o.id === priced.optionId)
  return option && options.length > 1 ? `${priced.currency} · ${option.name}` : priced.currency
}

// Cadences: how a price bills. Add-ons, tenant-level, are still priced per currency × cadence.

export const TERMS: {
  term: BillingPeriod
  label: string
  param: string
  per: string
  adverb: string
}[] = [
  { term: BillingPeriod.MONTHLY, label: 'Monthly', param: 'monthly', per: 'month', adverb: 'monthly' },
  { term: BillingPeriod.QUARTERLY, label: 'Quarterly', param: 'quarterly', per: 'quarter', adverb: 'quarterly' },
  { term: BillingPeriod.SEMIANNUAL, label: 'Semiannual', param: 'semiannual', per: 'half-year', adverb: 'every 6 months' },
  { term: BillingPeriod.ANNUAL, label: 'Annual', param: 'annual', per: 'year', adverb: 'yearly' },
]

const termOf = (term: BillingPeriod) => TERMS.find(t => t.term === term) ?? TERMS[0]
export const termLabel = (term: BillingPeriod) => termOf(term).label
export const termPer = (term: BillingPeriod) => termOf(term).per
/** "monthly", "yearly": how often a price on this cadence is billed. */
export const termAdverb = (term: BillingPeriod) => termOf(term).adverb
export const termParam = (term: BillingPeriod) => termOf(term).param
export const termFromParam = (param: string | null | undefined) =>
  TERMS.find(t => t.param === param)?.term

const FORM_TERMS: Record<number, Cadence> = {
  [BillingPeriod.MONTHLY]: 'MONTHLY',
  [BillingPeriod.QUARTERLY]: 'QUARTERLY',
  [BillingPeriod.SEMIANNUAL]: 'SEMIANNUAL',
  [BillingPeriod.ANNUAL]: 'ANNUAL',
}

/** The proto cadence as the charge form's `term` value. */
export const termFormValue = (term: BillingPeriod): Cadence => FORM_TERMS[term] ?? 'MONTHLY'

/** The charge form's `term` value as the proto cadence. */
export const termFromFormValue = (value: Cadence | undefined): BillingPeriod =>
  TERMS.find(t => FORM_TERMS[t.term] === value)?.term ?? BillingPeriod.MONTHLY

const PERIODIC = ['ratePricing', 'slotPricing', 'capacityPricing', 'extraRecurringPricing']

/** Billed every period, on its cadence; usage and one-time prices are not. */
export const hasTerm = (price: Pick<Price, 'pricing'>) =>
  PERIODIC.includes(price.pricing.case ?? '') ||
  (price.pricing.case === 'creditPricing' && !price.pricing.value.once)

/** An add-on's prices of one exact cell: capacity levels are several prices in one cell. */
export const cellPrices = <P extends Priced>(
  prices: P[],
  currency: string,
  term: BillingPeriod
): P[] => prices.filter(p => p.currency === currency && (!hasTerm(p) || p.cadence === term))

export type Cell<P extends Priced = Price> =
  | { kind: 'included' }
  | { kind: 'own'; prices: P[] }
  | { kind: 'missing' }

const exactCell = <P extends Priced>(prices: P[], currency: string, term: BillingPeriod): Cell<P> => {
  const own = cellPrices(prices, currency, term)
  return own.length ? { kind: 'own', prices: own } : { kind: 'missing' }
}

/** The one cadence an add-on's recurring prices share: it bills on it whatever the period. */
export function singleCadence(
  component: { prices: Priced[] },
  currency: string
): BillingPeriod | undefined {
  const cadences = new Set(component.prices.filter(hasTerm).map(p => p.cadence))
  const single = cadences.size === 1 ? [...cadences][0] : undefined
  return single !== undefined &&
    component.prices.some(p => p.currency === currency && hasTerm(p))
    ? single
    : undefined
}

/** What an add-on bills in a currency on a cadence; one on a single cadence bills it on every one. */
export function resolveCell<P extends Priced>(
  component: { prices: P[] },
  currency: string,
  term: BillingPeriod,
  { exact = false }: { exact?: boolean } = {}
): Cell<P> {
  if (component.prices.length === 0) return { kind: 'included' }
  const cell = exactCell(component.prices, currency, term)
  if (cell.kind !== 'missing' || exact) return cell
  const single = singleCadence(component, currency)
  return single === undefined || single === term
    ? cell
    : exactCell(component.prices, currency, single)
}
