import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { THIS_PLAN } from '@/features/plans/trial/trialTerms'
import { OfferSchema } from '@/rpc/api/offers/v1/offers_pb'
import { TrialEligibility } from '@/rpc/api/plans/v1/models_pb'

import { grantToProto } from './grants'
import {
  offerChips,
  offerFormSchema,
  offerSummary,
  toOfferForm,
  toOfferRequest,
  toStartOfferInline,
} from './offers'

import type { GrantForm } from './grants'
import type { OfferForm } from './offers'

const credit: GrantForm = {
  key: 'g1',
  kind: 'credit',
  window: { anchor: 'start', duration: 'days', count: 30 },
  currency: 'USD',
  amount: '50',
  scope: 'customer',
}

const storedOffer = () => {
  const grant = grantToProto(credit)
  return create(OfferSchema, {
    id: 'ofr_1',
    name: 'Launch',
    description: 'Spring launch',
    trial: { durationDays: 14, cardRequired: false },
    grants: [{ id: 'g1', window: grant.window, definition: grant.definition as never }],
    eligibility: TrialEligibility.PLAN,
    standalone: true,
  })
}

describe('offer form', () => {
  it('starts a new offer without a trial, given once per offer', () => {
    const form = toOfferForm()
    expect(form).toMatchObject({
      name: '',
      hasTrial: false,
      grants: [],
      eligibility: String(TrialEligibility.OFFER),
      days: 14,
      featuresOf: THIS_PLAN,
    })
  })

  it('reads an offer with its trial and grants', () => {
    const form = toOfferForm(storedOffer())
    expect(form).toMatchObject({
      name: 'Launch',
      description: 'Spring launch',
      hasTrial: true,
      days: 14,
      card: 'optional',
      eligibility: String(TrialEligibility.PLAN),
    })
    expect(form.grants).toEqual([credit])
  })

  it('asks a trial or a grant', () => {
    const empty = offerFormSchema.safeParse({ ...toOfferForm(), name: 'Empty' })
    expect(empty.success).toBe(false)
    expect(empty.error?.issues.map(issue => issue.path.join('.'))).toContain('grants')
    expect(
      offerFormSchema.safeParse({ ...toOfferForm(), name: 'Credit', grants: [credit] }).success
    ).toBe(true)
    expect(offerFormSchema.safeParse({ ...toOfferForm(), name: ' ', grants: [credit] }).success).toBe(
      false
    )
  })

  it('sends the trial only when it has one, and the eligibility as its enum', () => {
    const form: OfferForm = { ...toOfferForm(storedOffer()), description: '  ' }
    const request = toOfferRequest(form)
    expect(request).toMatchObject({
      name: 'Launch',
      description: undefined,
      eligibility: TrialEligibility.PLAN,
      trial: { durationDays: 14, cardRequired: false },
    })
    expect(request.grants).toEqual([grantToProto(credit)])
    expect(toOfferRequest({ ...form, hasTrial: false }).trial).toBeUndefined()
  })

  it('sends a plan’s own start offer, its trial for a free plan asking no card', () => {
    const form = toOfferForm(storedOffer())
    const inline = toStartOfferInline({ ...form, card: 'required' }, [credit], true)
    expect(inline.trial).toMatchObject({ durationDays: 14, cardRequired: false })
    expect(inline.grants).toEqual([grantToProto(credit)])
    expect(toStartOfferInline(undefined, [], false)).toEqual({ trial: undefined, grants: [] })
  })
})

describe('offer chips', () => {
  it('reads the trial first, then each grant', () => {
    const offer = storedOffer()
    expect(offerChips(offer).map(chip => chip.label)).toEqual([
      'Trial 14 d',
      expect.stringMatching(/^Credit .*50.* · 30 d$/),
    ])
    expect(offerChips(offer)[0].tone).toBe('info')
    expect(offerSummary({ grants: [] })).toBe('Nothing')
  })
})
