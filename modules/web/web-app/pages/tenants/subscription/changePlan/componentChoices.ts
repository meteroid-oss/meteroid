import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'
import type { SubscriptionComponent } from '@/rpc/api/subscriptions/v1/models_pb'

// The cadence is the option's, chosen once for the plan (`ChangePlanState.option`).
export interface ComponentChoice {
  committedCapacity?: bigint
  initialSlotCount?: number
}

export interface ComponentParameters extends ComponentChoice {
  componentId: string
}

interface Slots {
  minSlots?: number
  maxSlots?: number
}

/** An added component with a price to pick: each axis is listed only when it offers a choice. */
export interface ConfigurableComponent {
  /** Its prices are the chosen option's cell (see `inOption`). */
  component: PriceComponent
  capacities: bigint[]
  slot?: Slots
  choice: ComponentChoice
}

// Legacy (v1) components carry id-less prices and never match by product on the server.
const isMatched = (component: PriceComponent, currentProductIds: Set<string>) =>
  Boolean(component.productId) &&
  component.prices.some(p => p.id) &&
  currentProductIds.has(component.productId!)

/** The target components the server reports as added, in the server's order. */
export const addedComponents = (
  targets: PriceComponent[],
  current: SubscriptionComponent[]
): PriceComponent[] => {
  const currentProductIds = new Set(current.flatMap(c => (c.productId ? [c.productId] : [])))
  return targets.filter(t => !isMatched(t, currentProductIds))
}

const capacityOf = (price: Price) =>
  price.pricing.case === 'capacityPricing' ? price.pricing.value.included : undefined

const seats = ({ minSlots = 0, maxSlots }: Slots, chosen: number | undefined) =>
  Math.min(Math.max(chosen ?? Math.max(minSlots, 1), minSlots), maxSlots ?? Infinity)

const configure = (
  component: PriceComponent,
  chosen: ComponentChoice | undefined
): ConfigurableComponent | undefined => {
  const capacities = [
    ...new Set(component.prices.map(capacityOf).filter((c): c is bigint => c !== undefined)),
  ].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0))
  const committedCapacity =
    chosen?.committedCapacity !== undefined && capacities.includes(chosen.committedCapacity)
      ? chosen.committedCapacity
      : capacities[0]

  const slotPrice = component.prices.find(p => p.pricing.case === 'slotPricing')
  const slot =
    slotPrice?.pricing.case === 'slotPricing'
      ? { minSlots: slotPrice.pricing.value.minSlots, maxSlots: slotPrice.pricing.value.maxSlots }
      : undefined
  const initialSlotCount = slot && seats(slot, chosen?.initialSlotCount)

  if (capacities.length < 2 && !slot) return undefined
  return {
    component,
    capacities: capacities.length > 1 ? capacities : [],
    slot,
    choice: {
      committedCapacity: capacities.length > 1 ? committedCapacity : undefined,
      initialSlotCount,
    },
  }
}

/**
 * Those of the `added` components that need a capacity tier or seat count, each with the
 * operator's choice or its default: the lowest tier, the minimum seats (at least 1).
 */
export const configureAdded = (
  added: PriceComponent[],
  choices: Record<string, ComponentChoice>
): ConfigurableComponent[] => added.flatMap(c => configure(c, choices[c.id]) ?? [])

export const configurableComponents = (
  targets: PriceComponent[],
  current: SubscriptionComponent[],
  choices: Record<string, ComponentChoice>
): ConfigurableComponent[] => configureAdded(addedComponents(targets, current), choices)

/** The target components the server matches to a held one by product, in the server's order. */
export const matchedComponents = (
  targets: PriceComponent[],
  current: SubscriptionComponent[]
): PriceComponent[] => {
  const currentProductIds = new Set(current.flatMap(c => (c.productId ? [c.productId] : [])))
  return targets.filter(t => isMatched(t, currentProductIds))
}

const heldCapacity = (held: SubscriptionComponent | undefined) =>
  held?.fee?.fee.case === 'capacity' ? held.fee.fee.value.included : undefined

/**
 * Those of the `matched` components sold at several levels, each with the operator's level, else
 * the one held when the target sells it, else the lowest level above it, else the highest: the
 * server ports the level held only when the target has it.
 */
export const configureMatched = (
  matched: PriceComponent[],
  current: SubscriptionComponent[],
  choices: Record<string, ComponentChoice>
): ConfigurableComponent[] =>
  matched.flatMap(component => {
    const capacities = [
      ...new Set(component.prices.map(capacityOf).filter((c): c is bigint => c !== undefined)),
    ].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0))
    if (capacities.length < 2) return []
    const chosen = choices[component.id]?.committedCapacity
    const held = heldCapacity(current.find(c => c.productId === component.productId))
    const committedCapacity =
      chosen !== undefined && capacities.includes(chosen)
        ? chosen
        : held !== undefined
          ? (capacities.find(c => c >= held) ?? capacities[capacities.length - 1])
          : capacities[0]
    return [{ component, capacities, choice: { committedCapacity } }]
  })

export const toParameterizedComponents = (
  configurable: ConfigurableComponent[]
): ComponentParameters[] =>
  configurable.map(({ component, choice }) => ({ componentId: component.id, ...choice }))

export const parametersKey = (params: ComponentParameters[]): string =>
  params
    .map(p => `${p.componentId}:${p.committedCapacity ?? ''}:${p.initialSlotCount ?? ''}`)
    .join('|')
