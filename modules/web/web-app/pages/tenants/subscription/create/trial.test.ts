import { Code, ConnectError } from '@connectrpc/connect'
import { describe, expect, it } from 'vitest'

import { THIS_PLAN } from '@/features/plans/trial/trialTerms'

import { offerRequest, startOfferLabel, trialLabel, trialRefusal, trialRequest } from './trial'

describe('trialRequest', () => {
  it('asks the plan’s trial explicitly', () => {
    expect(trialRequest({ trialMode: 'plan' })).toEqual({ trial: { case: 'plan', value: true } })
  })

  it('asks none', () => {
    expect(trialRequest({ trialMode: 'none' })).toEqual({ trial: { case: 'none', value: true } })
  })

  it('leaves a catalog offer’s own trial to the offer named', () => {
    expect(
      trialRequest({ trialMode: 'plan', startOfferMode: 'catalog', startOfferId: 'ofr_1' })
    ).toBeUndefined()
    expect(trialRequest({ trialMode: 'none', startOfferMode: 'catalog' })).toEqual({
      trial: { case: 'none', value: true },
    })
  })

  it('sends a custom trial by days or end date, with its features, and no freeness', () => {
    expect(
      trialRequest({ trialMode: 'custom', trialDuration: 30, trialFeaturesOf: THIS_PLAN })
    ).toEqual({
      trial: { case: 'custom', value: { length: { case: 'days', value: 30 }, ownFeatures: true } },
    })
    expect(
      trialRequest({
        trialMode: 'custom',
        trialDuration: 30,
        trialUntil: new Date(2026, 11, 15),
        trialFeaturesOf: 'plan_pro',
      })
    ).toEqual({
      trial: {
        case: 'custom',
        value: { length: { case: 'until', value: '2026-12-15' }, trialPlanId: 'plan_pro' },
      },
    })
    expect(trialRequest({ trialMode: 'custom', trialDuration: 7 })).toEqual({
      trial: { case: 'custom', value: { length: { case: 'days', value: 7 } } },
    })
  })
})

describe('offerRequest', () => {
  it('asks the plan’s start offer explicitly by default', () => {
    expect(offerRequest({})).toEqual({ offer: { case: 'plan', value: true } })
    expect(offerRequest({ startOfferMode: 'plan' })).toEqual({ offer: { case: 'plan', value: true } })
  })

  it('asks none, or a catalog offer once picked', () => {
    expect(offerRequest({ startOfferMode: 'none' })).toEqual({ offer: { case: 'none', value: true } })
    expect(offerRequest({ startOfferMode: 'catalog', startOfferId: 'ofr_1' })).toEqual({
      offer: { case: 'offerId', value: 'ofr_1' },
    })
    expect(offerRequest({ startOfferMode: 'catalog' })).toBeUndefined()
  })
})

describe('startOfferLabel', () => {
  it('names the choice', () => {
    expect(startOfferLabel({ trialMode: 'plan' })).toBe("The plan's")
    expect(startOfferLabel({ trialMode: 'plan', startOfferMode: 'none' })).toBe('None')
    expect(startOfferLabel({ trialMode: 'plan', startOfferMode: 'catalog' }, 'Launch')).toBe(
      'Launch'
    )
  })
})

describe('trialLabel', () => {
  it('names the choice', () => {
    expect(trialLabel({ trialMode: 'plan' })).toBe("The plan's")
    expect(trialLabel({ trialMode: 'plan', startOfferMode: 'catalog' })).toBe("The offer's")
    expect(trialLabel({ trialMode: 'none' })).toBe('None')
    expect(
      trialLabel({ trialMode: 'custom', trialDuration: 1, trialFeaturesOf: 'plan_pro' }, () => 'Pro')
    ).toBe('1 day · Pro features')
    expect(trialLabel({ trialMode: 'custom', trialUntil: new Date(2026, 11, 15) })).toBe(
      'Until Dec 15, 2026'
    )
  })
})

describe('trialRefusal', () => {
  it('reads the server’s refusal of a trial already had', () => {
    const refused = new ConnectError(
      'the customer has already had this trial (Pro trial)',
      Code.InvalidArgument
    )
    expect(trialRefusal(refused)).toBe('the customer has already had this trial (Pro trial)')
    const offerRefused = new ConnectError(
      'the customer has already had this offer (Launch)',
      Code.FailedPrecondition
    )
    expect(trialRefusal(offerRefused)).toBe('the customer has already had this offer (Launch)')
    expect(trialRefusal(new ConnectError('no such plan', Code.InvalidArgument))).toBeUndefined()
    expect(trialRefusal(null)).toBeUndefined()
  })
})
