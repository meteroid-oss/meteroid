import { z } from 'zod'

import {
  checkTrialTerms,
  toTrialForm,
  toTrialTerms,
  trialTermsShape,
} from '@/features/plans/trial/trialTerms'
import { TrialEligibility } from '@/rpc/api/plans/v1/models_pb'

import { grantChip, grantToProto, grantsFromProto } from './grants'

import type { GrantChip, GrantForm, GrantNames } from './grants'
import type { TrialTermsForm } from '@/features/plans/trial/trialTerms'
import type {
  CreateOfferRequestSchema,
  Grant,
  Offer,
  SetPlanStartOfferRequest_InlineSchema,
} from '@/rpc/api/offers/v1/offers_pb'
import type { TrialTerms } from '@/rpc/api/plans/v1/models_pb'
import type { MessageInitShape } from '@bufbuild/protobuf'

export const ELIGIBILITY_OPTIONS = [
  TrialEligibility.OFFER,
  TrialEligibility.PLAN,
  TrialEligibility.PRODUCT_FAMILY,
  TrialEligibility.NONE,
]

/** The trial in a chip: "Trial 14 d". */
export const trialChip = (trial: TrialTerms): GrantChip => ({
  key: 'trial',
  label: `Trial ${trial.durationDays} d`,
  tone: 'info',
})

/** What an offer gives at a glance: its trial, then each grant. */
export const offerChips = (
  offer: { trial?: TrialTerms; grants: Grant[] },
  names: GrantNames = {}
): GrantChip[] => [
  ...(offer.trial ? [trialChip(offer.trial)] : []),
  ...grantsFromProto(offer.grants).map(grant => grantChip(grant, names)),
]

export const offerSummary = (offer: { trial?: TrialTerms; grants: Grant[] }, names: GrantNames = {}) =>
  offerChips(offer, names)
    .map(chip => chip.label)
    .join(' · ') || 'Nothing'

export const offerFormSchema = z
  .object({
    ...trialTermsShape,
    name: z.string().trim().min(1, 'Name it').max(200, 'At most 200 characters'),
    description: z.string().trim().max(1000, 'At most 1000 characters').optional(),
    hasTrial: z.boolean(),
    grants: z.array(z.custom<GrantForm>()),
    eligibility: z.string(),
  })
  .superRefine((value, ctx) => {
    if (value.hasTrial) checkTrialTerms(value, ctx)
    if (!value.hasTrial && value.grants.length === 0) {
      ctx.addIssue({ code: 'custom', message: 'Give a trial or a grant', path: ['grants'] })
    }
  })

export type OfferForm = z.output<typeof offerFormSchema>

export const toOfferForm = (offer?: Offer): OfferForm => ({
  ...toTrialForm(offer?.trial),
  name: offer?.name ?? '',
  description: offer?.description ?? '',
  hasTrial: offer ? !!offer.trial : false,
  grants: offer ? grantsFromProto(offer.grants) : [],
  eligibility: String(offer?.eligibility ?? TrialEligibility.OFFER),
})

/** The create request; an update sends the same fields with the offer's id. */
export const toOfferRequest = (form: OfferForm): MessageInitShape<typeof CreateOfferRequestSchema> => ({
  name: form.name.trim(),
  description: form.description?.trim() || undefined,
  trial: form.hasTrial ? toTrialTerms(form, false) : undefined,
  grants: form.grants.map(grantToProto),
  eligibility: Number(form.eligibility) as TrialEligibility,
})

/** A plan's own start offer: its trial, when it has one, and its grants. */
export const toStartOfferInline = (
  trial: TrialTermsForm | undefined,
  grants: GrantForm[],
  isFreePlan: boolean
): MessageInitShape<typeof SetPlanStartOfferRequest_InlineSchema> => ({
  trial: trial ? toTrialTerms(trial, isFreePlan) : undefined,
  grants: grants.map(grantToProto),
})
