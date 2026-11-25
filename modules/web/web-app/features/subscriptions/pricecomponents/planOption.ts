import { optionCell, pricedCurrencies, soldOptions } from '@/features/plans/currencies/priceCells'

import type { PlanOption } from '@/rpc/api/plans/v1/models_pb'
import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'

type Option = Pick<PlanOption, 'id' | 'name' | 'alias' | 'position'>

/** How a request names an option: its alias (the server takes the id as well). */
export const optionHandle = (option: Pick<Option, 'id' | 'alias'>) => option.alias || option.id

export const byPosition = <O extends Option>(options: O[]): O[] =>
  [...options].sort((a, b) => a.position - b.position)

export const findOption = <O extends Option>(options: O[], handle: string | undefined) =>
  handle ? options.find(o => o.alias === handle || o.id === handle) : undefined

/** The first option sold in `currency`, else the first one: the server's default. */
export const defaultOption = <O extends Option>(
  options: O[],
  components: PriceComponent[],
  currency: string
): O | undefined => {
  const ordered = byPosition(options)
  return soldOptions(ordered, components, currency)[0] ?? ordered[0]
}

/** The option `handle` names when the version has it, else the default one in `currency`. */
export const chosenOption = <O extends Option>(
  options: O[],
  components: PriceComponent[],
  currency: string,
  handle: string | undefined
): O | undefined => findOption(options, handle) ?? defaultOption(options, components, currency)

/**
 * The option and currency a new subscription is sold in, as the server resolves them: the option
 * asked, else the first sold; in the customer's currency when sold there, else the plan's.
 */
export function newSubscriptionSale<O extends Option>(
  options: O[],
  components: PriceComponent[],
  { customerCurrency, planCurrency, handle }: {
    customerCurrency?: string
    planCurrency: string
    handle?: string
  }
): { option?: O; currency: string } {
  const ordered = byPosition(options)
  const named = findOption(ordered, handle)
  if (!ordered.length || components.every(c => c.prices.length === 0)) {
    return { option: named ?? ordered[0], currency: planCurrency }
  }
  const candidates = [
    ...new Set(
      [customerCurrency, planCurrency, ...pricedCurrencies(components)]
        .filter((c): c is string => Boolean(c?.trim()))
        .map(c => c.trim().toUpperCase())
    ),
  ]
  if (named) {
    const currency = candidates.find(c => soldOptions([named], components, c).length > 0)
    return { option: named, currency: currency ?? planCurrency }
  }
  for (const currency of candidates) {
    const [first] = soldOptions(ordered, components, currency)
    if (first) return { option: first, currency }
  }
  return { option: ordered[0], currency: planCurrency }
}

/**
 * The charges as bought in `option`: each one's prices are that option's cell in `currency`, so
 * the price, the capacity levels and the cadence shown are the ones billed. A charge linked to no
 * option (made before options) keeps its prices.
 */
export const inOption = (
  components: PriceComponent[],
  option: Pick<Option, 'id'> | undefined,
  currency: string
): PriceComponent[] =>
  option
    ? components.map(c =>
        c.optionPrices.length ? { ...c, prices: optionCell(c, option.id, currency) } : c
      )
    : components
