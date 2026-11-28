import { describe, expect, it } from 'vitest'

import { QuoteTrialEndAction } from '@/rpc/api/quotes/v1/models_pb'

import { quoteTrialLabel, trialEndToForm, trialEndToProto } from './quoteTrial'

describe('trialEndToForm', () => {
  it('reads MANUAL as manual and anything else as convert', () => {
    expect(trialEndToForm(QuoteTrialEndAction.MANUAL)).toBe('manual')
    expect(trialEndToForm(QuoteTrialEndAction.CONVERT)).toBe('convert')
    expect(trialEndToForm(undefined)).toBe('convert')
  })
})

describe('trialEndToProto', () => {
  it('sends MANUAL only with a trial', () => {
    expect(trialEndToProto(14, 'manual')).toBe(QuoteTrialEndAction.MANUAL)
    expect(trialEndToProto(0, 'manual')).toBe(QuoteTrialEndAction.CONVERT)
    expect(trialEndToProto(undefined, 'manual')).toBe(QuoteTrialEndAction.CONVERT)
    expect(trialEndToProto(14, 'convert')).toBe(QuoteTrialEndAction.CONVERT)
    expect(trialEndToProto(14, undefined)).toBe(QuoteTrialEndAction.CONVERT)
  })
})

describe('quoteTrialLabel', () => {
  it('is undefined without a trial', () => {
    expect(quoteTrialLabel({ days: 0, manual: true })).toBeUndefined()
    expect(quoteTrialLabel({})).toBeUndefined()
  })

  it('states the days and the freeness as recorded', () => {
    expect(quoteTrialLabel({ days: 14, free: true })).toBe('14 days, free')
    expect(quoteTrialLabel({ days: 14, free: false })).toBe('14 days, billed')
    expect(quoteTrialLabel({ days: 14 })).toBe('14 days')
  })

  it('names a trial decided at the end', () => {
    expect(quoteTrialLabel({ days: 30, free: true, manual: true })).toBe(
      '30 days, free · Decide at the end'
    )
  })
})
