import { type MessageInitShape, create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  CardExtraSchema,
  CardExtra_Kind,
  CardPriceSchema,
  PricingCardSchema,
} from '@/rpc/api/pricingpage/v1/pricingpage_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import {
  type PriceDisplay,
  bestSaving,
  cadenceOptionLabel,
  ctaLabelOf,
  extraLabel,
  formatAmount,
  optionSavingLabel,
  priceDisplay,
  usageLabel,
} from './cardPrice'
import { fromHighlights, moveItem, toHighlights } from './highlights'

const L = 'en-US'

const card = (init: MessageInitShape<typeof PricingCardSchema>) =>
  create(PricingCardSchema, { planId: 'p', planName: 'Pro', ...init })

/** The display with its lines as the phrases they read. */
const read = (display: PriceDisplay) =>
  display.kind === 'priced'
    ? { ...display, extras: display.extras.map(line => line.text) }
    : display

describe('formatAmount', () => {
  it('drops the decimals of whole amounts', () => {
    expect(formatAmount('149.00', 'USD', L)).toBe('$149')
    expect(formatAmount('1490', 'USD', L)).toBe('$1,490')
  })

  it('keeps the currency precision, and more for unit prices', () => {
    expect(formatAmount('72.5', 'USD', L)).toBe('$72.50')
    expect(formatAmount('0.002', 'USD', L)).toBe('$0.002')
    expect(formatAmount('1500', 'JPY', L)).toBe('¥1,500')
  })

  it('shows a credit unit code after the amount', () => {
    expect(formatAmount('20', 'TOKENS', L)).toBe('20 TOKENS')
  })
})

describe('priceDisplay', () => {
  it('reads Free for a free card', () => {
    expect(priceDisplay(card({ free: true }), 'USD', L)).toEqual({
      kind: 'free',
    })
  })

  it('names the currency a plan is not sold in', () => {
    expect(priceDisplay(card({}), 'EUR', L)).toEqual({
      kind: 'unavailable',
      label: 'Not offered in EUR',
    })
  })

  it('notes a plan read in its default option', () => {
    const price = create(CardPriceSchema, { amount: '50', period: BillingPeriod.MONTHLY })
    expect(read(priceDisplay(card({ price, optionNote: 'Billed monthly' }), 'USD', L))).toEqual({
      kind: 'priced',
      amount: '$50',
      per: '/ month',
      saving: undefined,
      extras: [],
      note: 'Billed monthly',
    })
    const plain = priceDisplay(card({ price }), 'USD', L)
    expect(plain.kind === 'priced' ? plain.note : 'not priced').toBeUndefined()
  })

  it('shows a monthly price with its extras', () => {
    const price = create(CardPriceSchema, {
      amount: '149',
      period: BillingPeriod.MONTHLY,
      extras: [
        create(CardExtraSchema, { kind: CardExtra_Kind.PER_SEAT, name: 'Seats', amount: '12' }),
        create(CardExtraSchema, { kind: CardExtra_Kind.USAGE, name: 'API calls' }),
        create(CardExtraSchema, { kind: CardExtra_Kind.USAGE, name: 'Storage' }),
        create(CardExtraSchema, { kind: CardExtra_Kind.ONE_TIME, name: 'Setup', amount: '500' }),
      ],
    })
    expect(read(priceDisplay(card({ price }), 'USD', L))).toEqual({
      kind: 'priced',
      amount: '$149',
      per: '/ month',
      saving: undefined,
      extras: ['+ $12 / seat', '+ API calls', '+ Storage', '+ $500 once'],
    })
  })

  it('leads with the seat price when nothing fixed is due', () => {
    const price = create(CardPriceSchema, {
      amount: '0',
      period: BillingPeriod.MONTHLY,
      extras: [
        create(CardExtraSchema, { kind: CardExtra_Kind.PER_SEAT, name: 'Seats', amount: '14' }),
        create(CardExtraSchema, {
          kind: CardExtra_Kind.USAGE,
          name: 'AI Assist Actions',
          unit: 'AI Assist Action',
          amount: '0.02',
        }),
      ],
    })
    expect(read(priceDisplay(card({ price }), 'EUR', L))).toMatchObject({
      kind: 'priced',
      amount: '€14',
      per: '/ seat / month',
      saving: undefined,
      extras: ['+ €0.02 / AI Assist Action'],
    })
  })

  it('leads with the usage rate of a usage-only card', () => {
    const price = create(CardPriceSchema, {
      amount: '0',
      extras: [
        create(CardExtraSchema, {
          kind: CardExtra_Kind.USAGE,
          name: 'Tokens',
          unit: '1M tokens',
          amount: '3',
        }),
      ],
    })
    expect(read(priceDisplay(card({ price }), 'USD', L))).toMatchObject({
      kind: 'priced',
      amount: '$3',
      per: '/ 1M tokens',
      saving: undefined,
      extras: [],
    })
  })

  it('leads with the charge the server marks, the fixed amount beside it', () => {
    const seats = create(CardExtraSchema, {
      kind: CardExtra_Kind.PER_SEAT,
      name: 'Seats',
      amount: '12',
      lead: true,
    })
    const price = create(CardPriceSchema, {
      amount: '49',
      period: BillingPeriod.MONTHLY,
      extras: [
        seats,
        create(CardExtraSchema, { kind: CardExtra_Kind.USAGE, name: 'API calls' }),
      ],
    })
    expect(read(priceDisplay(card({ price }), 'USD', L))).toEqual({
      kind: 'priced',
      amount: '$12',
      per: '/ seat / month',
      saving: undefined,
      extras: ['+ $49 / mo base price', '+ API calls'],
      lead: seats,
    })
  })

  it('reads usage without one rate by its name', () => {
    const price = create(CardPriceSchema, {
      amount: '29',
      period: BillingPeriod.MONTHLY,
      extras: [
        create(CardExtraSchema, {
          kind: CardExtra_Kind.USAGE,
          name: 'Events',
          unit: 'event',
          amount: '0.001',
        }),
        create(CardExtraSchema, { kind: CardExtra_Kind.USAGE }),
        create(CardExtraSchema, { kind: CardExtra_Kind.USAGE, name: 'Storage' }),
      ],
    })
    expect(read(priceDisplay(card({ price }), 'USD', L))).toMatchObject({
      extras: ['+ $0.001 / event', '+ usage', '+ Storage'],
    })
    expect(usageLabel({ name: 'LLM tokens' })).toBe('+ LLM tokens')
    expect(usageLabel({ name: '' })).toBe('+ usage')
  })

  it('reads Pro led by its tiered seats, the base price and the rest in the card order', () => {
    const seats = create(CardExtraSchema, {
      kind: CardExtra_Kind.PER_SEAT,
      name: 'Seats',
      amount: '12',
      unit: 'seat',
      included: 3n,
      lead: true,
    })
    const price = create(CardPriceSchema, {
      amount: '99',
      period: BillingPeriod.MONTHLY,
      extras: [
        seats,
        create(CardExtraSchema, { kind: CardExtra_Kind.BASE, amount: '99' }),
        create(CardExtraSchema, {
          kind: CardExtra_Kind.RECURRING,
          name: 'Support hours',
          amount: '600',
          period: BillingPeriod.QUARTERLY,
        }),
        create(CardExtraSchema, {
          kind: CardExtra_Kind.USAGE,
          name: 'Events',
          amount: '0.0004',
          unit: 'event',
          included: 1000000n,
        }),
        create(CardExtraSchema, { kind: CardExtra_Kind.USAGE, name: 'LLM tokens' }),
      ],
    })
    const display = priceDisplay(card({ price }), 'USD', L)
    expect(read(display)).toEqual({
      kind: 'priced',
      amount: '$12',
      per: '/ seat / month',
      included: '3 seats included',
      saving: undefined,
      extras: [
        '+ $99 / mo base price',
        '+ $600 / qtr Support hours',
        '+ $0.0004 / event (1,000,000 events included)',
        '+ LLM tokens',
      ],
      note: undefined,
      lead: seats,
    })
    // Each line in parts: its figure, what it is, the units it includes in short.
    expect(display.kind === 'priced' && display.extras).toEqual([
      { text: '+ $99 / mo base price', figure: '$99 / mo', label: 'base price' },
      { text: '+ $600 / qtr Support hours', figure: '$600 / qtr', label: 'Support hours' },
      {
        text: '+ $0.0004 / event (1,000,000 events included)',
        figure: '$0.0004 / event',
        included: '1M included',
      },
      { text: '+ LLM tokens', label: 'LLM tokens' },
    ])
  })

  it('shortens an included count only when exact', () => {
    const usage = (included: bigint) =>
      create(CardPriceSchema, {
        amount: '0',
        extras: [
          create(CardExtraSchema, {
            kind: CardExtra_Kind.USAGE,
            amount: '1',
            unit: 'call',
            lead: true,
          }),
          create(CardExtraSchema, {
            kind: CardExtra_Kind.USAGE,
            amount: '0.01',
            unit: 'event',
            included,
          }),
        ],
      })
    const includedOf = (included: bigint) => {
      const display = priceDisplay(card({ price: usage(included) }), 'USD', L)
      return display.kind === 'priced' ? display.extras[0]?.included : undefined
    }
    expect(includedOf(12_500n)).toBe('12.5K included')
    expect(includedOf(1_234_567n)).toBe('1,234,567 included')
    expect(includedOf(5_000n)).toBe('5,000 included')
  })

  it('reads the base price first when it leads, wherever its item is', () => {
    const price = create(CardPriceSchema, {
      amount: '99',
      period: BillingPeriod.MONTHLY,
      extras: [
        create(CardExtraSchema, { kind: CardExtra_Kind.PER_SEAT, amount: '12' }),
        create(CardExtraSchema, { kind: CardExtra_Kind.BASE, amount: '99', lead: true }),
      ],
    })
    const display = read(priceDisplay(card({ price }), 'USD', L))
    expect(display.kind === 'priced' && [display.amount, display.per, display.extras]).toEqual([
      '$99',
      '/ month',
      ['+ $12 / seat'],
    ])
  })

  it('reads tiers from their lowest rate and packages by their size', () => {
    const tiered = create(CardExtraSchema, {
      kind: CardExtra_Kind.PER_SEAT,
      amount: '8',
      fromLowest: true,
      lead: true,
    })
    const price = create(CardPriceSchema, {
      amount: '0',
      period: BillingPeriod.MONTHLY,
      extras: [tiered],
    })
    const display = priceDisplay(card({ price }), 'USD', L)
    expect(display.kind === 'priced' && [display.from, display.amount, display.per]).toEqual([
      true,
      '$8',
      '/ seat / month',
    ])
    const pack = create(CardExtraSchema, {
      kind: CardExtra_Kind.USAGE,
      amount: '30',
      unit: 'export',
      blockSize: 10n,
    })
    expect(extraLabel(pack, 'USD', L)).toBe('+ $30 / 10 exports')
    expect(extraLabel(create(CardExtraSchema, { ...tiered, lead: false }), 'USD', L)).toBe(
      '+ from $8 / seat'
    )
  })

  it('shows a yearly price with its monthly equivalent and saving', () => {
    const price = create(CardPriceSchema, {
      amount: '1490',
      period: BillingPeriod.ANNUAL,
      monthlyEquivalent: '124.17',
      savingPercent: 17,
    })
    expect(read(priceDisplay(card({ price }), 'USD', L))).toEqual({
      kind: 'priced',
      amount: '$124.17',
      per: '/ month',
      billed: '$1,490 billed yearly',
      saving: 'Save 17%',
      extras: [],
    })
  })
})

describe('extraLabel', () => {
  it('uses the unit when there is one', () => {
    const extra = create(CardExtraSchema, {
      kind: CardExtra_Kind.USAGE,
      name: 'Events',
      amount: '0.002',
      unit: 'event',
    })
    expect(extraLabel(extra, 'USD', L)).toBe('+ $0.002 / event')
  })
})

describe('saving label', () => {
  const yearly = (savingPercent?: number) =>
    card({
      price: create(CardPriceSchema, { amount: '1', period: BillingPeriod.ANNUAL, savingPercent }),
    })

  it('takes the best saving among the cards', () => {
    expect(bestSaving([yearly(10), yearly(), card({}), yearly(17)])).toBe(17)
    expect(bestSaving([card({ free: true })])).toBe(0)
  })

  it('reads the saving on the yearly option only', () => {
    expect(cadenceOptionLabel(BillingPeriod.ANNUAL, 17)).toBe('Yearly · save up to 17%')
    expect(cadenceOptionLabel(BillingPeriod.ANNUAL, 0)).toBe('Yearly')
    expect(cadenceOptionLabel(BillingPeriod.MONTHLY, 17)).toBe('Monthly')
  })
})

describe('option saving', () => {
  const priced = (period: BillingPeriod, savingPercent?: number) =>
    card({ price: create(CardPriceSchema, { amount: '1', period, savingPercent }) })

  it('reads whole months free on a yearly option', () => {
    expect(optionSavingLabel([priced(BillingPeriod.ANNUAL, 17), card({ free: true })])).toBe(
      '2 months free'
    )
    const uneven = [priced(BillingPeriod.ANNUAL, 10), priced(BillingPeriod.ANNUAL, 17)]
    expect(optionSavingLabel(uneven)).toBe('save up to 17%')
    expect(optionSavingLabel([priced(BillingPeriod.MONTHLY)])).toBeUndefined()
  })

  it('leaves out a card read in its default option', () => {
    const fallback = card({
      price: create(CardPriceSchema, {
        amount: '1',
        period: BillingPeriod.ANNUAL,
        savingPercent: 30,
      }),
      optionNote: 'Annual only',
    })
    expect(optionSavingLabel([priced(BillingPeriod.ANNUAL, 17), fallback])).toBe('2 months free')
    expect(optionSavingLabel([fallback])).toBeUndefined()
    expect(bestSaving([fallback])).toBe(0)
  })
})

describe('card copy', () => {
  it('defaults the button by trial', () => {
    expect(ctaLabelOf(card({}))).toBe('Get started')
    expect(ctaLabelOf(card({ trialDays: 14 }))).toBe('Start a 14-day trial')
    expect(ctaLabelOf(card({ ctaLabel: 'Contact sales', trialDays: 14 }))).toBe('Contact sales')
  })

  it('contacts sales for a sales-led plan', () => {
    expect(ctaLabelOf(card({ salesLed: true, trialDays: 14 }))).toBe('Contact sales')
    expect(ctaLabelOf(card({ salesLed: true, ctaLabel: 'Talk to us' }))).toBe('Talk to us')
  })

  it('reads a hidden price as custom pricing', () => {
    expect(priceDisplay(card({ salesLed: true, hidePrice: true }), 'USD', L)).toEqual({
      kind: 'custom',
    })
  })
})

describe('highlights', () => {
  it('saves trimmed lines and drops blank ones', () => {
    const drafts = fromHighlights([
      { item: { case: 'featureId', value: 'f1' } },
      { item: { case: 'text', value: '  Priority support ' } },
      { item: { case: 'text', value: '  ' } },
    ])
    expect(toHighlights(drafts)).toEqual([
      { item: { case: 'featureId', value: 'f1' } },
      { item: { case: 'text', value: 'Priority support' } },
    ])
  })

  it('moves a row within the list only', () => {
    expect(moveItem(['a', 'b', 'c'], 2, -1)).toEqual(['a', 'c', 'b'])
    const list = ['a', 'b']
    expect(moveItem(list, 0, -1)).toBe(list)
    expect(moveItem(list, 1, 1)).toBe(list)
  })
})
