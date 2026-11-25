import { type MessageInitShape, create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  CardExtra_Kind,
  type CardLevelsSchema,
  PricingCardSchema,
} from '@/rpc/api/pricingpage/v1/pricingpage_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import {
  cardAtLevel,
  levelReading,
  levelStops,
  levelsCountMoney,
  overageLabel,
  signupParams,
} from './cardLevels'

const L = 'en-US'
const PAGE = { currency: 'USD', option: 'monthly' }

/** Volume: $29 platform fee, then $199 for $50K managed revenue, $479 for $125K, $899 for $250K,
 * 0.5% above the level; the card reads its lowest level. */
const volume = (
  levels: Partial<MessageInitShape<typeof CardLevelsSchema>> = {},
  init: Partial<MessageInitShape<typeof PricingCardSchema>> = {}
) =>
  create(PricingCardSchema, {
    planId: 'volume',
    planName: 'Volume',
    options: ['monthly'],
    price: {
      amount: '228',
      period: BillingPeriod.MONTHLY,
      fromLowest: true,
      savingPercent: 10,
      extras: [
        { kind: CardExtra_Kind.BASE, amount: '228', lead: true },
        {
          kind: CardExtra_Kind.USAGE,
          name: 'Managed revenue',
          amount: '0.005',
          unit: 'USD',
          levels: true,
        },
        { kind: CardExtra_Kind.ONE_TIME, name: 'Onboarding', amount: '500' },
      ],
      levels: {
        name: 'Managed revenue',
        metricName: 'Managed revenue',
        unitSingular: 'USD',
        unitPlural: 'USD',
        levels: [
          { included: 50000n, amount: '228', overageRate: '0.005' },
          { included: 125000n, amount: '508', overageRate: '0.005' },
          { included: 250000n, amount: '928', overageRate: '0.005' },
        ],
        more: true,
        moreContactUrl: 'https://example.com/sales',
        morePlanName: 'Enterprise',
        ...levels,
      } as MessageInitShape<typeof CardLevelsSchema>,
    },
    ...init,
  } as MessageInitShape<typeof PricingCardSchema>)

/** The same levels counting events. */
const events = {
  metricName: 'Events',
  unitSingular: 'event',
  unitPlural: 'events',
  levels: [
    { included: 50000n, amount: '228', overageRate: '0.002' },
    { included: 1500000n, amount: '508', overageRate: '0.002' },
  ],
  more: false,
}

describe('levelStops', () => {
  it('reads money-valued levels as amounts, then More', () => {
    const levels = volume().price!.levels!
    expect(levelStops(levels, 'USD', L).map(stop => stop.label)).toEqual([
      '$50K',
      '$125K',
      '$250K',
      'More',
    ])
    expect(levelStops(levels, 'USD', L).map(stop => stop.level)).toEqual([0, 1, 2, undefined])
  })

  it('reads other levels as counts, without More when nothing sells by contacting sales', () => {
    const levels = volume(events).price!.levels!
    expect(levelStops(levels, 'USD', L).map(stop => stop.label)).toEqual(['50K', '1.5M'])
  })
})

describe('levelsCountMoney', () => {
  it('follows the plan editor: a unit named as the currency, unscaled', () => {
    const levels = (init: Partial<MessageInitShape<typeof CardLevelsSchema>>) =>
      volume(init).price!.levels!
    expect(levelsCountMoney(levels({}), 'USD')).toBe(true)
    expect(levelsCountMoney(levels({}), 'EUR')).toBe(false)
    expect(levelsCountMoney(levels({ unitSingular: 'dollar', unitPlural: 'dollars' }), 'USD')).toBe(
      false
    )
    expect(levelsCountMoney(levels({ unitConversionFactor: 100 }), 'USD')).toBe(false)
    expect(
      levelsCountMoney(levels({ unitSingular: undefined, unitPlural: undefined }), 'USD')
    ).toBe(false)
  })
})

describe('levelReading', () => {
  it('reads the price, the limit and the overage of the level picked', () => {
    const reading = levelReading(volume(), 1, PAGE, L)
    expect(reading?.price).toEqual({
      kind: 'priced',
      amount: '$508',
      per: '/ month',
      saving: undefined,
      extras: [{ text: '+ $500 once', figure: '$500', label: 'once' }],
      note: undefined,
    })
    expect(reading?.limit).toBe('Up to $125K managed revenue per month')
    expect(reading?.overage).toBe('0.5% overage on managed revenue above your plan limit')
  })

  it('keeps the saving at the lowest level only, the one compared', () => {
    const lowest = levelReading(volume(), 0, PAGE, L)?.price
    expect(lowest?.kind === 'priced' && lowest.saving).toBe('Save 10%')
    const higher = levelReading(volume(), 2, PAGE, L)?.price
    expect(higher?.kind === 'priced' && higher.saving).toBeUndefined()
  })

  it('passes the level on to sign-up as its committed capacity', () => {
    const reading = levelReading(volume(), 2, PAGE, L)
    expect(reading?.cta.kind).toBe('signup')
    const params = reading?.cta.kind === 'signup' ? reading.cta.params : undefined
    expect(params?.toString()).toBe('plan=volume&currency=USD&option=monthly&capacity=250000')
  })

  it('contacts sales past the largest level', () => {
    const reading = levelReading(volume(), 3, PAGE, L)
    expect(reading).toEqual({
      price: { kind: 'custom' },
      limit: 'More than $250K managed revenue per month',
      cta: { kind: 'contact', url: 'https://example.com/sales', planName: 'Enterprise' },
    })
  })

  it('reads counted usage in its unit, its overage per unit', () => {
    const reading = levelReading(volume(events), 1, PAGE, L)
    expect(reading?.limit).toBe('Up to 1.5M events per month')
    expect(reading?.overage).toBe('$0.002 per event above your plan limit')
    expect(levelReading(volume(events), 2, PAGE, L)).toBeUndefined()
  })

  it('names the metric when the unit has no name', () => {
    const reading = levelReading(
      volume({ ...events, metricName: 'API calls', unitSingular: undefined, unitPlural: undefined }),
      0,
      PAGE,
      L
    )
    expect(reading?.limit).toBe('Up to 50K API calls per month')
  })

  it('says nothing of an overage left off the card', () => {
    const card = volume({
      levels: [{ included: 50000n, amount: '228' }],
    })
    expect(levelReading(card, 0, PAGE, L)?.overage).toBeUndefined()
  })

  it('keeps a sales-led card contacting sales at every level', () => {
    const card = volume({}, { salesLed: true, contactUrl: 'https://example.com/volume' })
    expect(levelReading(card, 0, PAGE, L)?.cta).toEqual({
      kind: 'contact',
      url: 'https://example.com/volume',
    })
  })

  it('reads nothing for a card without levels', () => {
    const card = create(PricingCardSchema, {
      planId: 'pro',
      price: { amount: '49', period: BillingPeriod.MONTHLY },
    })
    expect(levelReading(card, 0, PAGE, L)).toBeUndefined()
  })
})

describe('cardAtLevel', () => {
  it('moves the amount and the base price, and leaves the overage to its line', () => {
    const card = volume()
    const at = cardAtLevel(card, card.price!.levels!.levels[1])
    expect(at.price?.amount).toBe('508')
    expect(at.price?.fromLowest).toBe(false)
    expect(at.price?.extras.map(extra => [extra.kind, extra.amount])).toEqual([
      [CardExtra_Kind.BASE, '508'],
      [CardExtra_Kind.ONE_TIME, '500'],
    ])
  })
})

describe('overageLabel', () => {
  it('reads a share of money-valued usage, a price per unit otherwise', () => {
    const money = volume().price!.levels!
    expect(overageLabel(money, money.levels[0], 'USD', L)).toBe(
      '0.5% overage on managed revenue above your plan limit'
    )
    const counted = volume(events).price!.levels!
    expect(overageLabel(counted, counted.levels[0], 'USD', L)).toBe(
      '$0.002 per event above your plan limit'
    )
  })
})

describe('signupParams', () => {
  it('passes the plan, currency and option it sells, and a level as capacity', () => {
    const card = volume()
    expect(signupParams(card, PAGE).toString()).toBe('plan=volume&currency=USD&option=monthly')
    expect(signupParams(card, { currency: 'USD', option: 'annual' }).toString()).toBe(
      'plan=volume&currency=USD'
    )
    expect(signupParams(card, PAGE, card.price!.levels!.levels[0]).get('capacity')).toBe('50000')
  })
})
