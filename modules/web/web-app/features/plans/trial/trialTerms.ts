import { z } from 'zod'

import {
  TrialAtEnd_Kind,
  TrialEligibility,
  TrialWithoutCard_Kind,
} from '@/rpc/api/plans/v1/models_pb'

import type { TrialTerms, TrialTermsSchema } from '@/rpc/api/plans/v1/models_pb'
import type { MessageInitShape } from '@bufbuild/protobuf'

// Select items can't carry an empty value.
export const THIS_PLAN = 'current'

export const AT_END = ['convert', 'manual', 'cancel', 'switch'] as const
export const WITHOUT_CARD = ['await_payment', 'cancel', 'switch'] as const
export type AtEnd = (typeof AT_END)[number]
export type WithoutCard = (typeof WITHOUT_CARD)[number]

const AT_END_KIND: Record<AtEnd, TrialAtEnd_Kind> = {
  convert: TrialAtEnd_Kind.CONVERT,
  manual: TrialAtEnd_Kind.MANUAL,
  cancel: TrialAtEnd_Kind.CANCEL,
  switch: TrialAtEnd_Kind.SWITCH,
}

const WITHOUT_CARD_KIND: Record<WithoutCard, TrialWithoutCard_Kind> = {
  await_payment: TrialWithoutCard_Kind.AWAIT_PAYMENT,
  cancel: TrialWithoutCard_Kind.CANCEL,
  switch: TrialWithoutCard_Kind.SWITCH,
}

export const atEndOf = (kind: TrialAtEnd_Kind | undefined): AtEnd =>
  AT_END.find(key => AT_END_KIND[key] === kind) ?? 'convert'

export const withoutCardOf = (kind: TrialWithoutCard_Kind | undefined): WithoutCard =>
  WITHOUT_CARD.find(key => WITHOUT_CARD_KIND[key] === kind) ?? 'await_payment'

export const trialTermsShape = {
  days: z.preprocess(
    value => (value === '' || value === null ? undefined : value),
    z
      .number({ error: 'Enter a number of days' })
      .int('Enter a whole number of days')
      .min(1, 'From 1 to 3650 days')
      .max(3650, 'From 1 to 3650 days')
  ),
  /** THIS_PLAN, or the plan whose features the trial gives. */
  featuresOf: z.string(),
  card: z.enum(['required', 'optional']),
  atEnd: z.enum(AT_END),
  atEndPlanVersionId: z.string().optional(),
  withoutCard: z.enum(WITHOUT_CARD),
  withoutCardPlanVersionId: z.string().optional(),
}

type TrialTermsValues = {
  atEnd: AtEnd
  atEndPlanVersionId?: string
  withoutCard: WithoutCard
  withoutCardPlanVersionId?: string
}

/** A switch names its plan; added with `superRefine` to any schema holding the terms' fields. */
export const checkTrialTerms = (value: TrialTermsValues, ctx: z.RefinementCtx) => {
  if (value.atEnd === 'switch' && !value.atEndPlanVersionId) {
    ctx.addIssue({ code: 'custom', message: 'Choose a plan', path: ['atEndPlanVersionId'] })
  }
  if (
    value.atEnd === 'convert' &&
    value.withoutCard === 'switch' &&
    !value.withoutCardPlanVersionId
  ) {
    ctx.addIssue({ code: 'custom', message: 'Choose a plan', path: ['withoutCardPlanVersionId'] })
  }
}

export const trialTermsSchema = z.object(trialTermsShape).superRefine(checkTrialTerms)

export type TrialTermsForm = z.output<typeof trialTermsSchema>

export const toTrialForm = (terms?: TrialTerms): TrialTermsForm => ({
  days: terms?.durationDays ?? 14,
  featuresOf: terms?.trialPlanId ?? THIS_PLAN,
  card: (terms?.cardRequired ?? true) ? 'required' : 'optional',
  atEnd: atEndOf(terms?.atEnd?.kind),
  atEndPlanVersionId: terms?.atEnd?.planVersionId,
  withoutCard: withoutCardOf(terms?.withoutCard?.kind),
  withoutCardPlanVersionId: terms?.withoutCard?.planVersionId,
})

/** A free plan's trial (a reverse trial) asks no card and stays on the plan at its end. */
export const toTrialTerms = (
  form: TrialTermsForm,
  isFreePlan: boolean
): MessageInitShape<typeof TrialTermsSchema> => {
  const atEnd: AtEnd = isFreePlan ? 'convert' : form.atEnd
  const withoutCard: WithoutCard =
    isFreePlan || atEnd !== 'convert' ? 'await_payment' : form.withoutCard
  return {
    durationDays: form.days,
    trialPlanId: form.featuresOf === THIS_PLAN ? undefined : form.featuresOf,
    cardRequired: !isFreePlan && form.card === 'required',
    atEnd: {
      kind: AT_END_KIND[atEnd],
      planVersionId: atEnd === 'switch' ? form.atEndPlanVersionId : undefined,
    },
    withoutCard: {
      kind: WITHOUT_CARD_KIND[withoutCard],
      planVersionId: withoutCard === 'switch' ? form.withoutCardPlanVersionId : undefined,
    },
  }
}

export interface TrialChip {
  label: string
  tone: 'info' | 'secondary' | 'warning'
}

/** Names a plan by its id or one of its versions' ids. */
export type PlanNamer = (id?: string) => string | undefined

export const daysLabel = (days: number) => `${days} ${days === 1 ? 'day' : 'days'}`

/** The trial at a glance: "14 days", "Card optional", "No card → awaits payment". */
export const trialChips = (
  form: TrialTermsForm,
  { isFreePlan, planName, ownName }: { isFreePlan: boolean; planName: PlanNamer; ownName?: string }
): TrialChip[] => {
  const name = (id?: string) => planName(id) ?? 'another plan'
  const chips: TrialChip[] = [{ label: daysLabel(form.days), tone: 'info' }]
  if (form.featuresOf !== THIS_PLAN) {
    chips.push({ label: `${name(form.featuresOf)} features`, tone: 'secondary' })
  }
  if (isFreePlan) {
    chips.push({ label: `Stays on ${ownName ?? 'this plan'}`, tone: 'secondary' })
    return chips
  }
  chips.push({
    label: form.card === 'required' ? 'Card required' : 'Card optional',
    tone: 'secondary',
  })
  switch (form.atEnd) {
    case 'manual':
      chips.push({ label: 'Decided at the end', tone: 'secondary' })
      break
    case 'cancel':
      chips.push({ label: 'Cancels at the end', tone: 'secondary' })
      break
    case 'switch':
      chips.push({ label: `Switches to ${name(form.atEndPlanVersionId)}`, tone: 'secondary' })
      break
    case 'convert':
      chips.push({
        label:
          form.withoutCard === 'await_payment'
            ? 'No card → awaits payment'
            : form.withoutCard === 'cancel'
              ? 'No card → cancels'
              : `No card → switches to ${name(form.withoutCardPlanVersionId)}`,
        tone: 'warning',
      })
      break
  }
  return chips
}

export const ELIGIBILITY_LABELS: Record<TrialEligibility, string> = {
  [TrialEligibility.NONE]: 'No limit',
  [TrialEligibility.OFFER]: 'Once per customer · this offer',
  [TrialEligibility.PLAN]: 'Once per customer · plan',
  [TrialEligibility.PRODUCT_FAMILY]: 'Once per customer · product family',
}
