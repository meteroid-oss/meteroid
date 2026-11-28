import { THIS_PLAN, daysLabel } from '@/features/plans/trial/trialTerms'
import { mapDatev2 } from '@/lib/mapping'
import { rpcErrorMessage } from '@/lib/rpcError'
import { formatDateOnly } from '@/lib/utils/date'

import type { PlanNamer } from '@/features/plans/trial/trialTerms'
import type { OfferRequestSchema, TrialRequestSchema } from '@/rpc/api/subscriptions/v1/models_pb'
import type { MessageInitShape } from '@bufbuild/protobuf'

/** The plan's start offer, none, or a catalog offer in its place (Enterprise). */
export type StartOfferMode = 'plan' | 'none' | 'catalog'

/** The start offer's trial (`plan`), none, or one of its own. */
export type TrialMode = 'plan' | 'none' | 'custom'

export interface TrialChoice {
  startOfferMode?: StartOfferMode
  startOfferId?: string
  trialMode: TrialMode
  trialDuration?: number
  trialUntil?: Date
  /** THIS_PLAN, another plan's id, or unset for the plan's trial's. */
  trialFeaturesOf?: string
}

/** The offer the creation asks for; the plan's is asked explicitly, never left to the default. */
export const offerRequest = (
  choice: Pick<TrialChoice, 'startOfferMode' | 'startOfferId'>
): MessageInitShape<typeof OfferRequestSchema> | undefined => {
  switch (choice.startOfferMode ?? 'plan') {
    case 'plan':
      return { offer: { case: 'plan', value: true } }
    case 'none':
      return { offer: { case: 'none', value: true } }
    case 'catalog':
      return choice.startOfferId ? { offer: { case: 'offerId', value: choice.startOfferId } } : undefined
  }
}

/**
 * The trial the creation asks for. The plan's is asked explicitly, refused when already had; a
 * catalog offer's is its own, asked by naming the offer.
 */
export const trialRequest = (
  choice: TrialChoice
): MessageInitShape<typeof TrialRequestSchema> | undefined => {
  switch (choice.trialMode) {
    case 'plan':
      return choice.startOfferMode === 'catalog' ? undefined : { trial: { case: 'plan', value: true } }
    case 'none':
      return { trial: { case: 'none', value: true } }
    case 'custom':
      return {
        trial: {
          case: 'custom',
          value: {
            length: choice.trialUntil
              ? { case: 'until', value: mapDatev2(choice.trialUntil) }
              : { case: 'days', value: choice.trialDuration ?? 0 },
            ...(choice.trialFeaturesOf === THIS_PLAN
              ? { ownFeatures: true }
              : choice.trialFeaturesOf
                ? { trialPlanId: choice.trialFeaturesOf }
                : {}),
          },
        },
      }
  }
}

export const startOfferLabel = (choice: TrialChoice, offerName?: string) => {
  switch (choice.startOfferMode ?? 'plan') {
    case 'plan':
      return "The plan's"
    case 'none':
      return 'None'
    case 'catalog':
      return offerName ?? 'Catalog offer'
  }
}

export const trialLabel = (choice: TrialChoice, planName: PlanNamer = () => undefined) => {
  switch (choice.trialMode) {
    case 'plan':
      return choice.startOfferMode === 'catalog' ? "The offer's" : "The plan's"
    case 'none':
      return 'None'
    case 'custom': {
      const length = choice.trialUntil
        ? `Until ${formatDateOnly(mapDatev2(choice.trialUntil))}`
        : daysLabel(choice.trialDuration ?? 0)
      const features =
        choice.trialFeaturesOf && choice.trialFeaturesOf !== THIS_PLAN
          ? planName(choice.trialFeaturesOf)
          : undefined
      return features ? `${length} · ${features} features` : length
    }
  }
}

/** The server's refusal of a trial or an offer the customer already had, else nothing. */
export const trialRefusal = (error: unknown): string | undefined => {
  if (!error) return undefined
  const message = rpcErrorMessage(error, '')
  return /already had this (trial|offer)/i.test(message) ? message : undefined
}
