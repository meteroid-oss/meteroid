import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  CardExtra_Kind,
  PricingCardSchema,
  PricingChargeSchema,
  PricingCharge_Kind,
} from '@/rpc/api/pricingpage/v1/pricingpage_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import { inOrder, movedOrder, orderRows } from './cardOrder'

const L = 'en-US'

const charges = [
  create(PricingChargeSchema, {
    productId: 'platform',
    name: 'Platform fee',
    kind: PricingCharge_Kind.FIXED,
    canLead: true,
  }),
  create(PricingChargeSchema, {
    productId: 'seats',
    name: 'Seats',
    kind: PricingCharge_Kind.SEATS,
    canLead: true,
  }),
  create(PricingChargeSchema, {
    productId: 'events',
    name: 'Events',
    kind: PricingCharge_Kind.USAGE,
    canLead: true,
    canHide: true,
  }),
  create(PricingChargeSchema, {
    productId: 'tokens',
    name: 'LLM tokens',
    kind: PricingCharge_Kind.USAGE,
    canHide: true,
  }),
]

/** Pro as the server reads it with seats first and LLM tokens left off. */
const pro = create(PricingCardSchema, {
  planId: 'pro',
  price: {
    amount: '99',
    period: BillingPeriod.MONTHLY,
    extras: [
      {
        kind: CardExtra_Kind.PER_SEAT,
        name: 'Seats',
        amount: '12',
        unit: 'seat',
        included: 3n,
        lead: true,
        productIds: ['seats'],
      },
      { kind: CardExtra_Kind.BASE, amount: '99', productIds: ['platform'] },
      {
        kind: CardExtra_Kind.USAGE,
        name: 'Events',
        amount: '0.0004',
        unit: 'event',
        productIds: ['events'],
      },
    ],
  },
})

describe('orderRows', () => {
  it('lists the card in its order, the base price as one row, a charge left off after', () => {
    const rows = orderRows(pro, charges, 'USD', ['tokens'], L)
    expect(rows.map(row => [row.id, row.name, row.reading, row.lead, row.hideable])).toEqual([
      ['seats', 'Seats', '$12 / seat (3 seats included)', true, false],
      ['platform', 'Base price', '$99 / mo base price', false, true],
      ['events', 'Events', '$0.0004 / event', false, true],
      ['tokens', 'LLM tokens', 'Not on the card', false, true],
    ])
    expect(rows[3].hidden).toBe(true)
  })

  it('lists the charges by kind before the plan is on the page', () => {
    const rows = orderRows(undefined, charges, 'USD', [], L)
    expect(rows.map(row => [row.name, row.reading])).toEqual([
      ['Platform fee', 'Fixed fee'],
      ['Seats', 'Per seat'],
      ['Events', 'Usage'],
      ['LLM tokens', 'Usage'],
    ])
  })
})

describe('inOrder', () => {
  it('follows the draft at once, the rows it does not place after as read', () => {
    const rows = orderRows(pro, charges, 'USD', ['tokens'], L)
    expect(inOrder(rows, ['events', 'platform']).map(row => row.id)).toEqual([
      'events',
      'platform',
      'seats',
      'tokens',
    ])
  })
})

describe('movedOrder', () => {
  it('saves every row in its new place, then what the card does not show', () => {
    const rows = orderRows(pro, charges, 'USD', ['tokens'], L)
    expect(movedOrder(rows, ['seats', 'annual-only'], 'platform', 'seats')).toEqual([
      'platform',
      'seats',
      'events',
      'tokens',
      'annual-only',
    ])
  })

  it('keeps the order when nothing moves', () => {
    const rows = orderRows(pro, charges, 'USD', [], L)
    const order = ['seats']
    expect(movedOrder(rows, order, 'seats', 'seats')).toBe(order)
    expect(movedOrder(rows, order, 'gone', 'seats')).toBe(order)
  })
})
