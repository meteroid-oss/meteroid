import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  EntitlementEffect,
  EntitlementLevel,
  EntitlementSchema,
  FeatureKind,
  FeatureSchema,
  LimitCountedBy,
} from '@/rpc/api/entitlements/v1/models_pb'
import { ResourceTypeSchema } from '@/rpc/api/resources/v1/models_pb'

import { EFFECT, rowPhrase, rowsLines } from './model'

const site = create(ResourceTypeSchema, {
  id: 'rt_site',
  code: 'site',
  name: 'Site',
  pluralName: 'Sites',
  keys: ['site_id'],
})
const types = new Map([[site.id, site]])
const feature = create(FeatureSchema, {
  id: 'f',
  name: 'Seats',
  kind: FeatureKind.LIMIT,
  limit: { countedBy: LimitCountedBy.APP },
  levelResourceTypeId: site.id,
})
const row = (limit: string, effect: EntitlementEffect, level: EntitlementLevel) =>
  create(EntitlementSchema, {
    featureId: feature.id,
    effect,
    level,
    value: { value: { case: 'meteredValue', value: { enabled: true, limit } } },
  })

describe("a plan's rows across all resources", () => {
  it('read as the level and the value, not as a cap', () => {
    const rows = [
      row('50', EntitlementEffect.SET, EntitlementLevel.FEATURE),
      row('500', EntitlementEffect.SET, EntitlementLevel.CUSTOMER),
    ]
    const lines = rowsLines(rows, feature, types, 'Pro')
    expect(lines).toEqual([
      { label: 'Each site on Pro', value: '50' },
      { label: 'Across all sites on Pro', value: '500' },
    ])
  })

  it('keep the cap wording for a Cap at row only', () => {
    const cap = row('500', EntitlementEffect.CAP_AT, EntitlementLevel.CUSTOMER)
    expect(rowPhrase(cap, feature)).toBe(`${EFFECT[EntitlementEffect.CAP_AT].label} 500`)
    expect(rowPhrase(row('500', EntitlementEffect.SET, EntitlementLevel.CUSTOMER), feature)).toBe('500')
  })
})
