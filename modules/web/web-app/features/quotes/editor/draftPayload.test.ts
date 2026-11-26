import { create, toBinary } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { newDraft } from '@/features/entitlements/rows/grantDraft'
import { EntitlementSpecSchema, FeatureKind, FeatureSchema } from '@/rpc/api/entitlements/v1/models_pb'
import { QuoteSectionPlacement } from '@/rpc/api/quotes/v1/models_pb'

import { decodeSnapshot, encodeSnapshot, toTemplatePayload } from './draftPayload'
import { ALL_CUSTOMER_ACTIONS, emptyQuoteV2State } from './quoteV2'

import type { EntitlementSpec } from '@/rpc/api/entitlements/v1/models_pb'

describe('decodeSnapshot', () => {
  it('allows every customer action for a payload written before they existed', () => {
    const { customerActions: _, ...legacyV2 } = emptyQuoteV2State()
    const snapshot = decodeSnapshot(JSON.stringify({ values: {}, v2: legacyV2 }))
    expect(snapshot?.v2.customerActions).toEqual(ALL_CUSTOMER_ACTIONS)
  })

  it('keeps the customer actions it was saved with', () => {
    const v2 = {
      ...emptyQuoteV2State(),
      customerActions: { questions: true, changeRequests: false, decline: false },
    }
    const snapshot = decodeSnapshot(
      encodeSnapshot({
        values: {},
        priceComponents: {
          components: { removed: [], parameterized: [], overridden: [], extra: [] },
        },
        addOns: [],
        coupons: [],
        pendingEntitlements: [],
        v2,
      })
    )
    expect(snapshot?.v2.customerActions).toEqual(v2.customerActions)
  })
})

describe('decodeSnapshot, legacy document fields', () => {
  it('turns the overview and terms into the first and last sections', () => {
    const v2 = {
      ...emptyQuoteV2State(),
      sections: [{ title: 'Scope', body: 'Work', placement: QuoteSectionPlacement.AFTER_PRICING }],
    }
    const snapshot = decodeSnapshot(
      JSON.stringify({
        values: { overview: 'Hello', terms_and_services: 'Net 30' },
        v2,
      })
    )
    expect(snapshot?.values).not.toHaveProperty('overview')
    expect(snapshot?.values).not.toHaveProperty('terms_and_services')
    expect(snapshot?.v2.sections).toEqual([
      { title: 'Overview', body: 'Hello', placement: QuoteSectionPlacement.BEFORE_PRICING },
      { title: 'Scope', body: 'Work', placement: QuoteSectionPlacement.AFTER_PRICING },
      { title: 'Terms & services', body: 'Net 30', placement: QuoteSectionPlacement.AFTER_PRICING },
    ])
  })

  it('adds no section for empty legacy fields', () => {
    const snapshot = decodeSnapshot(
      JSON.stringify({ values: { overview: ' ', terms_and_services: '' } })
    )
    expect(snapshot?.v2.sections).toEqual([])
  })
})

describe('the resource of a snapshot', () => {
  const snapshot = {
    values: {},
    priceComponents: {
      components: { removed: [], parameterized: [], overridden: [], extra: [] },
    },
    addOns: [],
    coupons: [],
    pendingEntitlements: [],
    v2: emptyQuoteV2State(),
    resourceId: 'r1',
  }

  it('survives an autosave', () => {
    expect(decodeSnapshot(encodeSnapshot(snapshot))?.resourceId).toBe('r1')
  })

  it('is left out of a template', () => {
    expect(decodeSnapshot(JSON.stringify(toTemplatePayload(snapshot)))?.resourceId).toBeUndefined()
  })
})

describe('the pending entitlements of a snapshot', () => {
  const base = {
    values: {},
    priceComponents: {
      components: { removed: [], parameterized: [], overridden: [], extra: [] },
    },
    addOns: [],
    coupons: [],
    v2: emptyQuoteV2State(),
  }
  const feature = create(FeatureSchema, { id: 'feat_1', name: 'Seats', code: 'seats', kind: FeatureKind.LIMIT })
  const bytes = (spec: EntitlementSpec) => toBinary(EntitlementSpecSchema, spec)

  it('survive an autosave', () => {
    const draft = newDraft(feature, { value: { case: 'meteredValue', value: { limit: '10', enabled: true } } })
    const decoded = decodeSnapshot(encodeSnapshot({ ...base, pendingEntitlements: [draft] }))
    expect(decoded?.pendingEntitlements).toHaveLength(1)
    expect(decoded?.pendingEntitlements[0].feature.name).toBe('Seats')
    expect(bytes(decoded!.pendingEntitlements[0].spec)).toEqual(bytes(draft.spec))
  })

  it('read the specs the old dialog stored, as it submitted them', () => {
    const legacy = [
      { featureId: 'feat_1', featureDisplayName: 'Seats', featureType: 'metered', limit: '5', meteredEnabled: false },
      { featureId: 'feat_2', featureDisplayName: 'Region', featureType: 'config', configKind: 'number', configNumber: ' 3 ' },
      { featureName: 'New one', featureDisplayName: 'New one', featureType: 'boolean', boolEnabled: true },
    ]
    const decoded = decodeSnapshot(JSON.stringify({ ...base, pendingEntitlements: legacy }))
    expect(decoded?.pendingEntitlements.map(d => d.feature.name)).toEqual(['Seats', 'Region'])
    expect(bytes(decoded!.pendingEntitlements[0].spec)).toEqual(
      bytes(
        create(EntitlementSpecSchema, {
          featureId: 'feat_1',
          value: { value: { case: 'meteredValue', value: { limit: '5', enabled: false } } },
        })
      )
    )
    expect(bytes(decoded!.pendingEntitlements[1].spec)).toEqual(
      bytes(
        create(EntitlementSpecSchema, {
          featureId: 'feat_2',
          value: { value: { case: 'configValue', value: { value: { case: 'number', value: '3' } } } },
        })
      )
    )
  })
})
