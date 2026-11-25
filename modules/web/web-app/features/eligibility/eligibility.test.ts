import { describe, expect, it } from 'vitest'

import {
  eligibilitySummary,
  emptyEligibility,
  fromDraft,
  isOpen,
  newCondition,
  toDraft,
} from './eligibility'

import type { Rule } from './eligibility'

const NAMES: Record<string, string> = { BR: 'Brazil', PT: 'Portugal', ES: 'Spain', FR: 'France' }
const name = (code: string) => NAMES[code] ?? code
const rule = (init: Partial<Rule>): Rule => ({ countries: [], properties: [], ...init })

describe('eligibilitySummary', () => {
  it('reads an empty rule as everyone', () => {
    expect(eligibilitySummary(undefined)).toBe('Everyone')
    expect(eligibilitySummary(rule({}))).toBe('Everyone')
  })

  it('says where', () => {
    expect(eligibilitySummary(rule({ countries: ['BR'] }), name)).toBe('Only customers in Brazil')
  })

  it('lists a few countries, counts many', () => {
    expect(eligibilitySummary(rule({ countries: ['BR', 'PT'] }), name)).toBe(
      'Only customers in Brazil or Portugal'
    )
    expect(eligibilitySummary(rule({ countries: ['BR', 'PT', 'ES', 'FR'] }), name)).toBe(
      'Only customers in 4 countries'
    )
  })

  it('joins the country and the properties', () => {
    expect(
      eligibilitySummary(
        rule({ countries: ['BR'], properties: [{ key: 'student', values: ['verified'] }] }),
        name
      )
    ).toBe('Only customers in Brazil whose student is verified')
    expect(
      eligibilitySummary(
        rule({
          properties: [
            { key: 'segment', values: ['startup', 'scaleup'] },
            { key: 'student', values: ['verified'] },
          ],
        })
      )
    ).toBe('Only customers whose segment is startup or scaleup and student is verified')
  })

  it('ignores a property row without values', () => {
    expect(eligibilitySummary(rule({ properties: [{ key: 'segment', values: [] }] }))).toBe(
      'Everyone'
    )
  })
})

describe('drafts', () => {
  it('saves a trimmed rule, dropping unfinished rows', () => {
    const saved = fromDraft({
      restricted: true,
      conditions: [
        { kind: 'country', countries: ['BR', 'BR'] },
        { kind: 'property', key: ' student ', values: [' verified ', ''] },
        { kind: 'property', key: '', values: ['x'] },
        { kind: 'property', key: 'tier', values: [] },
      ],
    })
    expect(saved).toEqual({
      countries: ['BR'],
      properties: [{ key: 'student', values: ['verified'] }],
    })
  })

  it('saves everyone as an empty rule, whatever rows were left', () => {
    const saved = fromDraft({
      restricted: false,
      conditions: [{ kind: 'country', countries: ['BR'] }],
    })
    expect(isOpen(saved)).toBe(true)
  })

  it('reads a missing rule as everyone, and a rule back as its rows', () => {
    expect(toDraft(undefined)).toEqual(emptyEligibility())
    const stored = rule({ countries: ['BR'], properties: [{ key: 'student', values: ['verified'] }] })
    expect(toDraft(stored)).toEqual({
      restricted: true,
      conditions: [
        { kind: 'country', countries: ['BR'] },
        { kind: 'property', key: 'student', values: ['verified'] },
      ],
    })
    expect(fromDraft(toDraft(stored))).toEqual(stored)
  })

  it('offers the country first, then a property', () => {
    expect(newCondition([]).kind).toBe('country')
    expect(newCondition([{ kind: 'country', countries: [] }]).kind).toBe('property')
  })
})
