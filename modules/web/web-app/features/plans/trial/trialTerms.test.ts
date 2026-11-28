import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  TrialAtEnd_Kind,
  TrialTermsSchema,
  TrialWithoutCard_Kind,
} from '@/rpc/api/plans/v1/models_pb'

import { THIS_PLAN, toTrialForm, toTrialTerms, trialChips, trialTermsSchema } from './trialTerms'

import type { TrialTermsForm } from './trialTerms'

const form = (patch: Partial<TrialTermsForm> = {}): TrialTermsForm => ({
  days: 14,
  featuresOf: THIS_PLAN,
  card: 'optional',
  atEnd: 'convert',
  withoutCard: 'await_payment',
  ...patch,
})

const NAMES: Record<string, string> = { plan_pro: 'Pro', pv_free: 'Free' }
const names = (id?: string) => (id ? NAMES[id] : undefined)

describe('toTrialForm', () => {
  it('defaults a new trial to 14 days, a card required, converting and awaiting payment', () => {
    expect(toTrialForm(undefined)).toEqual({
      days: 14,
      featuresOf: THIS_PLAN,
      card: 'required',
      atEnd: 'convert',
      atEndPlanVersionId: undefined,
      withoutCard: 'await_payment',
      withoutCardPlanVersionId: undefined,
    })
  })

  it('reads stored terms', () => {
    const terms = create(TrialTermsSchema, {
      durationDays: 30,
      trialPlanId: 'plan_pro',
      cardRequired: false,
      atEnd: { kind: TrialAtEnd_Kind.SWITCH, planVersionId: 'pv_free' },
      withoutCard: { kind: TrialWithoutCard_Kind.CANCEL },
    })
    expect(toTrialForm(terms)).toEqual({
      days: 30,
      featuresOf: 'plan_pro',
      card: 'optional',
      atEnd: 'switch',
      atEndPlanVersionId: 'pv_free',
      withoutCard: 'cancel',
      withoutCardPlanVersionId: undefined,
    })
  })
})

describe('toTrialTerms', () => {
  it('writes the terms with no freeness, a switch naming its plan only', () => {
    expect(
      toTrialTerms(
        form({ featuresOf: 'plan_pro', withoutCard: 'switch', withoutCardPlanVersionId: 'pv_free' }),
        false
      )
    ).toEqual({
      durationDays: 14,
      trialPlanId: 'plan_pro',
      cardRequired: false,
      atEnd: { kind: TrialAtEnd_Kind.CONVERT, planVersionId: undefined },
      withoutCard: { kind: TrialWithoutCard_Kind.SWITCH, planVersionId: 'pv_free' },
    })
  })

  it('drops what follows a missing card unless the trial converts', () => {
    const terms = toTrialTerms(
      form({ atEnd: 'manual', withoutCard: 'cancel', atEndPlanVersionId: 'pv_free' }),
      false
    )
    expect(terms.atEnd).toEqual({ kind: TrialAtEnd_Kind.MANUAL, planVersionId: undefined })
    expect(terms.withoutCard).toEqual({
      kind: TrialWithoutCard_Kind.AWAIT_PAYMENT,
      planVersionId: undefined,
    })
  })

  it('makes a free plan’s trial a reverse trial: no card, stays on the plan', () => {
    const terms = toTrialTerms(form({ card: 'required', atEnd: 'cancel' }), true)
    expect(terms.cardRequired).toBe(false)
    expect(terms.atEnd).toEqual({ kind: TrialAtEnd_Kind.CONVERT, planVersionId: undefined })
  })
})

describe('trialTermsSchema', () => {
  it('asks a switch for its plan', () => {
    expect(trialTermsSchema.safeParse(form({ atEnd: 'switch' })).success).toBe(false)
    expect(
      trialTermsSchema.safeParse(form({ atEnd: 'switch', atEndPlanVersionId: 'pv_free' })).success
    ).toBe(true)
    expect(trialTermsSchema.safeParse(form({ withoutCard: 'switch' })).success).toBe(false)
    expect(trialTermsSchema.safeParse(form({ atEnd: 'cancel', withoutCard: 'switch' })).success).toBe(
      true
    )
  })

  it('refuses an empty or out-of-range length', () => {
    expect(trialTermsSchema.safeParse({ ...form(), days: '' }).success).toBe(false)
    expect(trialTermsSchema.safeParse(form({ days: 0 })).success).toBe(false)
    expect(trialTermsSchema.safeParse(form({ days: 3651 })).success).toBe(false)
  })
})

describe('trialChips', () => {
  const labels = (f: TrialTermsForm, isFreePlan = false) =>
    trialChips(f, { isFreePlan, planName: names, ownName: 'Free' }).map(chip => chip.label)

  it('summarises a converting trial with what follows a missing card', () => {
    expect(labels(form())).toEqual(['14 days', 'Card optional', 'No card → awaits payment'])
    expect(labels(form({ card: 'required', withoutCard: 'cancel' }))).toEqual([
      '14 days',
      'Card required',
      'No card → cancels',
    ])
    expect(
      labels(form({ withoutCard: 'switch', withoutCardPlanVersionId: 'pv_free' }))
    ).toContain('No card → switches to Free')
  })

  it('names the other end actions', () => {
    expect(labels(form({ atEnd: 'manual' }))).toContain('Decided at the end')
    expect(labels(form({ atEnd: 'cancel' }))).toContain('Cancels at the end')
    expect(labels(form({ atEnd: 'switch', atEndPlanVersionId: 'pv_free' }))).toContain(
      'Switches to Free'
    )
  })

  it('reads a free plan’s trial as a reverse trial', () => {
    expect(labels(form({ days: 1, featuresOf: 'plan_pro' }), true)).toEqual([
      '1 day',
      'Pro features',
      'Stays on Free',
    ])
  })
})
