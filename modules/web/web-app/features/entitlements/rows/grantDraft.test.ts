import { create, toBinary } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  EntitlementSpecSchema,
  ResolvedEntitlementSchema,
} from '@/rpc/api/entitlements/v1/models_pb'

import { draftsToSpecs, inheritedDraft, withEnabled } from './grantDraft'

import type { MessageInitShape } from '@bufbuild/protobuf'

const resolved = (value: MessageInitShape<typeof ResolvedEntitlementSchema>['value']) =>
  create(ResolvedEntitlementSchema, { feature: { id: 'feat_1', name: 'Seats', code: 'seats' }, value })

// What the old creation flows submitted for a pinned row.
const spec = (value: MessageInitShape<typeof EntitlementSpecSchema>['value']) =>
  toBinary(EntitlementSpecSchema, create(EntitlementSpecSchema, { featureId: 'feat_1', value }))

const pinned = (r: ReturnType<typeof resolved>) =>
  toBinary(EntitlementSpecSchema, draftsToSpecs([inheritedDraft(r)])[0])

describe('pinning an inherited entitlement', () => {
  it('keeps a limit and its switch', () => {
    const r = resolved({ case: 'metered', value: { metricId: 'm', limit: '100', enabled: false } })
    expect(pinned(r)).toEqual(spec({ value: { case: 'meteredValue', value: { limit: '100', enabled: false } } }))
  })

  it('keeps an unlimited limit unlimited', () => {
    const r = resolved({ case: 'metered', value: { metricId: 'm', limit: '', enabled: true } })
    expect(pinned(r)).toEqual(spec({ value: { case: 'meteredValue', value: { enabled: true } } }))
  })

  it('normalizes config values as the old dialog did', () => {
    const number = resolved({ case: 'config', value: { value: { value: { case: 'number', value: ' 3 ' } } } })
    expect(pinned(number)).toEqual(
      spec({ value: { case: 'configValue', value: { value: { case: 'number', value: '3' } } } })
    )
    const flat = resolved({ case: 'config', value: { value: { value: { case: 'json', value: '{ "a": 1, "b": "x" }' } } } })
    expect(pinned(flat)).toEqual(
      spec({ value: { case: 'configValue', value: { value: { case: 'json', value: '{"a":1,"b":"x"}' } } } })
    )
    const nested = resolved({ case: 'config', value: { value: { value: { case: 'json', value: ' {"a":{"b":1}} ' } } } })
    expect(pinned(nested)).toEqual(
      spec({ value: { case: 'configValue', value: { value: { case: 'json', value: '{"a":{"b":1}}' } } } })
    )
  })

  it('switches a pinned value off without losing its limit', () => {
    const r = resolved({ case: 'metered', value: { metricId: 'm', limit: '100', enabled: true } })
    const off = draftsToSpecs([withEnabled(inheritedDraft(r), false)])[0]
    expect(toBinary(EntitlementSpecSchema, off)).toEqual(
      spec({ value: { case: 'meteredValue', value: { limit: '100', enabled: false } } })
    )
  })
})
