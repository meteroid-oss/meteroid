import { useState } from 'react'

import type { AvailableOption, AvailablePlan } from '@/rpc/portal/subscription/v1/subscription_pb'

// A plan's options ("Monthly", "Annual") are what a customer picks. Plans of a ladder name an
// option alike by its alias, so one switch picks the same option on every card.

/** A choice of the option switch: an alias, and the name the plans give it. */
export type OptionChoice = { alias: string; name: string }

/** The options some plan offers, as the switch lists them: the current plan's first. */
export const optionChoices = (plans: AvailablePlan[]): OptionChoice[] => {
  const current = plans.find(p => p.isCurrent)
  const ordered = current ? [current, ...plans.filter(p => p !== current)] : plans
  const choices = new Map<string, OptionChoice>()
  ordered.forEach(p =>
    p.options.forEach(o => {
      if (!choices.has(o.alias)) choices.set(o.alias, { alias: o.alias, name: o.name })
    })
  )
  return [...choices.values()]
}

/** The subscription's own option, else the current plan's first. */
export const defaultOption = (plans: AvailablePlan[]): string | undefined => {
  const current = plans.find(p => p.isCurrent)
  const own = current?.options.find(o => o.isCurrent)
  if (own) return own.alias
  return (current ?? plans.find(p => p.options.length > 0))?.options[0]?.alias
}

/** The plan's option `alias`; a plan with a single option offers it whatever is picked. */
export const variantFor = (
  plan: AvailablePlan,
  alias: string | undefined
): AvailableOption | undefined =>
  plan.options.find(o => o.alias === alias) ??
  (plan.options.length === 1 ? plan.options[0] : undefined)

/** The plan has options, but not the one shown. */
export const notOffered = (plan: AvailablePlan, option: AvailableOption | undefined) =>
  plan.options.length > 0 && !option

export const notOfferedLabel = (choice: OptionChoice | undefined) =>
  choice ? `No ${choice.name} option` : 'Not offered'

const choicePrice = (plan: AvailablePlan, option: AvailableOption | undefined) =>
  plan.options.length ? option?.price : plan.price

/** Another plan costing less than the current one on the same option; false when unknown. */
export const cheaperThanCurrent = (
  plans: AvailablePlan[],
  plan: AvailablePlan,
  option: AvailableOption | undefined,
  alias: string | undefined
) => {
  const current = plans.find(p => p.isCurrent)
  if (!current || plan.isCurrent) return false
  const price = choicePrice(plan, option)
  const currentPrice = choicePrice(current, variantFor(current, alias))
  return (
    !!price &&
    !!currentPrice &&
    price.cadence === currentPrice.cadence &&
    price.currency === currentPrice.currency &&
    price.amount < currentPrice.amount
  )
}

/** What the subscription already bills: the plan, and its option when the plan has options. */
export const isCurrentChoice = (plan: AvailablePlan, option: AvailableOption | undefined) =>
  plan.isCurrent && (option ? option.isCurrent : true)

/** The current plan on another of its options: a switch on the same plan. */
export const isPeriodSwitch = (plan: AvailablePlan, option: AvailableOption | undefined) =>
  plan.isCurrent && !!option && !option.isCurrent

export const periodSwitchLabel = (option: AvailableOption) => `Switch to ${option.name}`

/** The option's name when it says more than the switch does: a plan with several, off-switch. */
export const variantLabel = (
  plan: AvailablePlan,
  option: AvailableOption | undefined,
  shown?: string
) => {
  const name = option?.name.trim()
  if (!name || plan.options.length < 2 || option?.alias === shown) return undefined
  return name
}

/** The option the plan cards show, and whether the customer may switch it. */
export const usePlanOption = (plans: AvailablePlan[], periodSwitch: boolean) => {
  const [picked, setPicked] = useState<string>()
  const choices = optionChoices(plans)
  const showToggle = periodSwitch && choices.length >= 2
  const option = showToggle && picked !== undefined ? picked : defaultOption(plans)
  return { choices, option, setOption: setPicked, showToggle }
}
