import { describe, expect, it } from 'vitest'

import { QuoteSectionPlacement } from '@/rpc/api/quotes/v1/models_pb'

import { decodeSnapshot, encodeSnapshot } from './draftPayload'
import { ALL_CUSTOMER_ACTIONS, emptyQuoteV2State } from './quoteV2'

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
