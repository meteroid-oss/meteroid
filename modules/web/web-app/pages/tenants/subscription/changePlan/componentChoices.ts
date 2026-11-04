import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import { SubscriptionFeeBillingPeriod } from '@/rpc/api/subscriptions/v1/models_pb'

import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'
import type { Price } from '@/rpc/api/prices/v1/models_pb'
import type { SubscriptionComponent } from '@/rpc/api/subscriptions/v1/models_pb'

export interface ComponentChoice {
  billingPeriod?: BillingPeriod
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
  component: PriceComponent
  cadences: BillingPeriod[]
  capacities: bigint[]
  slot?: Slots
  choice: ComponentChoice
}

const CADENCE_MONTHS: Record<BillingPeriod, number> = {
  [BillingPeriod.MONTHLY]: 1,
  [BillingPeriod.QUARTERLY]: 3,
  [BillingPeriod.SEMIANNUAL]: 6,
  [BillingPeriod.ANNUAL]: 12,
}

const toCadence = (period: SubscriptionFeeBillingPeriod): BillingPeriod | undefined => {
  switch (period) {
    case SubscriptionFeeBillingPeriod.MONTHLY:
      return BillingPeriod.MONTHLY
    case SubscriptionFeeBillingPeriod.QUARTERLY:
      return BillingPeriod.QUARTERLY
    case SubscriptionFeeBillingPeriod.SEMIANNUAL:
      return BillingPeriod.SEMIANNUAL
    case SubscriptionFeeBillingPeriod.YEARLY:
      return BillingPeriod.ANNUAL
    default:
      return undefined
  }
}

const byMonths = (a: BillingPeriod, b: BillingPeriod) => CADENCE_MONTHS[a] - CADENCE_MONTHS[b]

/** The subscription's shortest recurring cadence, as the server's `Ported::subscription_cadence`. */
export const subscriptionCadence = (current: SubscriptionComponent[]): BillingPeriod | undefined =>
  current
    .map(c => toCadence(c.period))
    .filter((c): c is BillingPeriod => c !== undefined)
    .sort(byMonths)[0]

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
  defaultCadence: BillingPeriod | undefined,
  chosen: ComponentChoice | undefined
): ConfigurableComponent | undefined => {
  const cadences = [...new Set(component.prices.map(p => p.cadence))].sort(byMonths)
  const pick = <T>(options: T[], ...preferred: (T | undefined)[]) =>
    preferred.find(o => o !== undefined && options.includes(o)) ?? options[0]

  const billingPeriod = pick(cadences, chosen?.billingPeriod, defaultCadence)
  const atCadence =
    cadences.length > 1
      ? component.prices.filter(p => p.cadence === billingPeriod)
      : component.prices

  const capacities = [
    ...new Set(atCadence.map(capacityOf).filter((c): c is bigint => c !== undefined)),
  ].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0))
  const committedCapacity = pick(capacities, chosen?.committedCapacity)

  const slotPrice = atCadence.find(p => p.pricing.case === 'slotPricing')
  const slot =
    slotPrice?.pricing.case === 'slotPricing'
      ? { minSlots: slotPrice.pricing.value.minSlots, maxSlots: slotPrice.pricing.value.maxSlots }
      : undefined
  const initialSlotCount = slot && seats(slot, chosen?.initialSlotCount)

  if (cadences.length < 2 && capacities.length < 2 && !slot) return undefined
  return {
    component,
    cadences: cadences.length > 1 ? cadences : [],
    capacities: capacities.length > 1 ? capacities : [],
    slot,
    choice: {
      billingPeriod: cadences.length > 1 ? billingPeriod : undefined,
      committedCapacity: capacities.length > 1 ? committedCapacity : undefined,
      initialSlotCount,
    },
  }
}

/**
 * The added components that need a cadence, capacity tier or seat count, each with the operator's
 * choice or its default: the subscription's cadence, the lowest tier, the minimum seats (at least 1).
 */
export const configurableComponents = (
  targets: PriceComponent[],
  current: SubscriptionComponent[],
  choices: Record<string, ComponentChoice>
): ConfigurableComponent[] => {
  const cadence = subscriptionCadence(current)
  return addedComponents(targets, current).flatMap(c => configure(c, cadence, choices[c.id]) ?? [])
}

export const toParameterizedComponents = (
  configurable: ConfigurableComponent[]
): ComponentParameters[] =>
  configurable.map(({ component, choice }) => ({ componentId: component.id, ...choice }))

export const parametersKey = (params: ComponentParameters[]): string =>
  params
    .map(
      p =>
        `${p.componentId}:${p.billingPeriod ?? ''}:${p.committedCapacity ?? ''}:${p.initialSlotCount ?? ''}`
    )
    .join('|')
