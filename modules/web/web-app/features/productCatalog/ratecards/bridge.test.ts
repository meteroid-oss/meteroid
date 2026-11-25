import { describe, expect, it } from 'vitest'

import { RateCardUnmatched } from '@/rpc/api/ratecards/v1/models_pb'

import { cardRatesForPrice, priceRatesForCard } from './bridge'

const card = {
  dimensions: ['model', 'token_type'],
  unitSize: '1000000',
  onUnmatched: RateCardUnmatched.IGNORE,
  creditUnitId: undefined,
}
const rates = [
  { key: ['*', '*'], currency: 'USD', rate: '1' },
  { key: ['gpt-4o', 'output'], currency: 'USD', rate: '10' },
  { key: ['gpt-4o', 'input'], currency: 'USD', rate: '2.5', label: 'GPT-4o in' },
  { key: ['gpt-4o', 'input'], currency: 'EUR', rate: '2.3' },
]

describe('cardRatesForPrice', () => {
  it("copies the card's rates in the price's currency, any other last", () => {
    expect(cardRatesForPrice(card, rates, 'USD')).toEqual({
      dimensions: ['model', 'token_type'],
      keyRates: [
        { key: ['gpt-4o', 'input'], rate: '2.5', label: 'GPT-4o in' },
        { key: ['gpt-4o', 'output'], rate: '10' },
        { key: ['*', '*'], rate: '1' },
      ],
      unitSize: '1000000',
      onUnmatched: 'ignore',
    })
  })

  it('applies the markup to each rate', () => {
    const copied = cardRatesForPrice(card, rates, 'EUR', '-20')
    expect(copied?.keyRates).toEqual([{ key: ['gpt-4o', 'input'], rate: '1.84' }])
  })

  it('has nothing to copy without a rate in the currency', () => {
    expect(cardRatesForPrice(card, rates, 'GBP')).toBeUndefined()
  })

  it("copies a credit card's rates, which carry no currency", () => {
    const credits = { ...card, creditUnitId: 'cu-1' }
    const copied = cardRatesForPrice(credits, [{ key: ['a', '*'], rate: '3' }, ...rates], 'USD')
    expect(copied?.keyRates).toEqual([{ key: ['a', '*'], rate: '3' }])
  })
})

describe('priceRatesForCard', () => {
  it("sends the price's rates in its currency, an empty pattern matching any value", () => {
    expect(
      priceRatesForCard(
        [
          { key: ['gpt-4o', ''], rate: '2.5' },
          { key: ['*', '*'], rate: '' },
        ],
        'USD'
      )
    ).toEqual([{ key: ['gpt-4o', '*'], currency: 'USD', rate: '2.5' }])
  })

  it('sends rates in credits without a currency', () => {
    expect(priceRatesForCard([{ key: ['a'], rate: '3' }], 'USD', 'cu-1')).toEqual([
      { key: ['a'], currency: undefined, rate: '3' },
    ])
  })
})
