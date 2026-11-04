import { create } from '@bufbuild/protobuf'
import { timestampFromDate } from '@bufbuild/protobuf/wkt'
import { describe, expect, it } from 'vitest'

import {
  CreditAppliesTo,
  CreditGrantKind,
  CreditGroupSchema,
  CreditRolloverSchema,
} from '@/rpc/api/wallets/v1/models_pb'

import { creditGroupRows } from './creditGroups'

const usd = { currency: 'USD' }

describe('creditGroupRows', () => {
  it('words each group with its own amount, use, reset and rollover', () => {
    const [included, prepaid] = creditGroupRows(
      [
        create(CreditGroupSchema, {
          kind: CreditGrantKind.INCLUDED,
          subscriptionId: 'sub_pro',
          appliesTo: CreditAppliesTo.USAGE,
          label: 'Included with Pro',
          paysFor: 'Pays Pro usage',
          available: 55_000n,
          nextLapseAt: timestampFromDate(new Date('2026-11-01T00:00:00Z')),
          nextLapseAmount: 55_000n,
          rollover: create(CreditRolloverSchema, { maxAmount: 10_000n, term: '1 month' }),
        }),
        create(CreditGroupSchema, {
          kind: CreditGrantKind.PREPAID,
          appliesTo: CreditAppliesTo.ALL,
          label: 'Purchased credit',
          paysFor: 'Pays any invoice',
          available: 5_000n,
          upcoming: 2_000n,
        }),
      ],
      usd
    )
    expect(included.label).toBe('Included with Pro')
    expect(included.amount).toBe('$550.00')
    expect(included.paysFor).toBe('Pays Pro usage')
    expect(included.lapse).toMatch(/^Resets /)
    expect(included.rollover).toBe('Up to $100.00 rolls over for 1 month')
    expect(included.scheduled).toBeUndefined()

    expect(prepaid.amount).toBe('$50.00')
    expect(prepaid.scheduled).toBe('+ $20.00 scheduled')
    expect(prepaid.lapse).toBeUndefined()
    expect(prepaid.rollover).toBeUndefined()
    expect(prepaid.key).not.toBe(included.key)
  })

  it('reads a non-plan expiry as an expiry and an uncapped rollover as all of it rolling over', () => {
    const [bonus] = creditGroupRows(
      [
        create(CreditGroupSchema, {
          kind: CreditGrantKind.BONUS,
          label: 'Free credit',
          paysFor: 'Pays any invoice',
          available: 1_000n,
          nextLapseAt: timestampFromDate(new Date('2026-12-01T00:00:00Z')),
          rollover: create(CreditRolloverSchema, { term: '2 months' }),
        }),
      ],
      usd
    )
    expect(bonus.lapse).toMatch(/^Expires /)
    expect(bonus.rollover).toBe('All of it rolls over for 2 months')
  })
  it('names how much lapses when only part of a group does', () => {
    const lapse = (available: bigint, nextLapseAmount: bigint, denomination: typeof usd) =>
      creditGroupRows(
        [
          create(CreditGroupSchema, {
            kind: CreditGrantKind.PREPAID,
            available,
            nextLapseAmount,
            nextLapseAt: timestampFromDate(new Date('2027-10-01T00:00:00Z')),
          }),
        ],
        denomination
      )[0].lapse
    const credits = {
      currency: 'HAL',
      creditUnit: { name: 'credit', pluralName: 'credits', precision: 0 },
    }
    expect(lapse(5_000n, 520n, usd)).toMatch(/^\$5\.20 expires /)
    expect(lapse(5_000n, 5_000n, usd)).toMatch(/^Expires /)
    expect(lapse(5_520n, 520n, credits)).toMatch(/^520 credits expire /)
  })
  it('names the usage filter of the subscription a group belongs to', () => {
    const groups = [
      create(CreditGroupSchema, { kind: CreditGrantKind.INCLUDED, subscriptionId: 'sub_mobile' }),
      create(CreditGroupSchema, { kind: CreditGrantKind.PREPAID }),
    ]
    const [mobile, shared] = creditGroupRows(
      groups,
      usd,
      new Map([['sub_mobile', 'team = mobile']])
    )
    expect(mobile.usageFilter).toBe('team = mobile')
    expect(shared.usageFilter).toBeUndefined()
  })
  it("leaves out each of its grants' unbilled usage, as the grants table does", () => {
    const [included, prepaid] = creditGroupRows(
      [
        create(CreditGroupSchema, {
          kind: CreditGrantKind.INCLUDED,
          available: 562_000n,
          creditGrantIds: ['rollover', 'monthly'],
        }),
        create(CreditGroupSchema, {
          kind: CreditGrantKind.PREPAID,
          available: 200_000n,
          creditGrantIds: ['purchase'],
        }),
      ],
      usd,
      new Map(),
      new Map([
        ['rollover', 62_000n],
        ['monthly', 328_000n],
        ['elsewhere', 1_000n],
      ])
    )
    expect(included.amount).toBe('$1,720.00')
    expect(prepaid.amount).toBe('$2,000.00')
  })
})

describe('credit unit rollover wording', () => {
  const units = {
    currency: 'FAST_REQ',
    creditUnit: { name: 'fast request', pluralName: 'fast requests', precision: 0 },
  }
  const rollover = (maxAmount: bigint) =>
    creditGroupRows(
      [
        create(CreditGroupSchema, {
          kind: CreditGrantKind.INCLUDED,
          label: 'Included with Pro',
          available: 500n,
          rollover: create(CreditRolloverSchema, { maxAmount, term: '1 month' }),
        }),
      ],
      units
    )[0].rollover

  it('agrees the verb with the number of credits', () => {
    expect(rollover(250n)).toBe('Up to 250 fast requests roll over for 1 month')
    expect(rollover(1n)).toBe('Up to 1 fast request rolls over for 1 month')
  })
})
