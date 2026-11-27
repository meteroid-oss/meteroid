import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { EntitlementEffect, FeatureKind, FeatureSchema } from '@/rpc/api/entitlements/v1/models_pb'

import { rowToForm, specFromForm, untilOf } from './rowForm'

const day = (y: number, m: number, d: number) => new Date(y, m - 1, d)

describe('an override window', () => {
  it('counts its shortcuts in calendar months from its start', () => {
    const validFrom = day(2026, 1, 31)
    expect(untilOf({ ends: '1m', validFrom })).toEqual(day(2026, 2, 28))
    expect(untilOf({ ends: '3m', validFrom })).toEqual(day(2026, 4, 30))
    expect(untilOf({ ends: '12m', validFrom })).toEqual(day(2027, 1, 31))
  })

  it('counts from today without a start, and has no end when it never ends', () => {
    const today = new Date(2026, 5, 10, 15, 30)
    expect(untilOf({ ends: '1m' }, today)).toEqual(day(2026, 7, 10))
    expect(untilOf({ ends: 'never' }, today)).toBeUndefined()
  })

  it('takes a picked date at the start of its day', () => {
    expect(untilOf({ ends: 'date', validUntil: new Date(2026, 2, 1, 18) })).toEqual(day(2026, 3, 1))
  })
})

describe('a temporary override', () => {
  const feature = create(FeatureSchema, { id: 'feat_1', kind: FeatureKind.BOOLEAN })

  it('raises the value for a quarter from its start, with its reason', () => {
    const form = {
      ...rowToForm(feature, undefined, { level: 'feature', effect: EntitlementEffect.RAISE_TO }),
      ends: '3m' as const,
      validFrom: day(2026, 12, 1),
      reason: ' Pilot extension ',
    }
    const spec = specFromForm(feature, form)
    expect(spec.effect).toBe(EntitlementEffect.RAISE_TO)
    expect(spec.validFrom).toBe('2026-12-01T00:00:00.000Z')
    expect(spec.validUntil).toBe('2027-03-01T00:00:00.000Z')
    expect(spec.reason).toBe('Pilot extension')
  })
})
