import { describe, expect, it } from 'vitest'

import {
  acceptRateInput,
  changeOf,
  currencySymbol,
  dimensionName,
  emptyDraft,
  rateCardHeading,
  rateCardSummary,
  type Row,
} from './rateTable'

const tokens = { singular: 'token', plural: 'tokens' }

describe('rate card table', () => {
  it('names dimensions in words', () => {
    expect(dimensionName('token_type')).toBe('Token type')
    expect(dimensionName('tokenType')).toBe('Token type')
    expect(dimensionName('model')).toBe('Model')
  })

  it('says what the card prices, per what, by what', () => {
    const card = { dimensions: ['model', 'token_type'], unitSize: '1000000' }
    expect(rateCardHeading(card, tokens)).toBe(
      'What each token costs, per 1M tokens, by Model and Token type.'
    )
    expect(rateCardHeading({ dimensions: ['model'], unitSize: '1' }, undefined)).toBe(
      'What each unit costs, by Model.'
    )
  })

  it('takes digits and one decimal point only', () => {
    expect(acceptRateInput('2.50')).toBe('2.50')
    expect(acceptRateInput('2,5')).toBe('2.5')
    expect(acceptRateInput('')).toBe('')
    expect(acceptRateInput('ssss')).toBeUndefined()
    expect(acceptRateInput('1.2.3')).toBeUndefined()
    expect(acceptRateInput('-1')).toBeUndefined()
  })

  it('reads an empty key cell as any and a trailing point as a whole number', () => {
    const rows: Row[] = [{ id: 'any', key: ['*', '*'], anyOther: true, cells: {} }]
    const draft = {
      ...emptyDraft,
      added: [{ id: 'new', key: ['gpt-4o', ''] }],
      edits: { new: { USD: '2.' } },
    }
    expect(changeOf(rows, draft)).toMatchObject({
      rates: [{ key: ['gpt-4o', '*'], currency: 'USD', rate: '2' }],
      errors: {},
    })
    const anything = { ...draft, added: [{ id: 'new', key: ['', ''] }] }
    expect(changeOf(rows, anything).errors.new).toBeDefined()
  })

  it('sums up a card: its keys rated, in what, per what', () => {
    const card = { creditUnitId: undefined, unitSize: '1000000' }
    const rates = [
      { key: ['gpt-4o', 'input'], currency: 'USD', rate: '2.5' },
      { key: ['gpt-4o', 'input'], currency: 'EUR', rate: '2.3' },
      { key: ['gpt-4o', 'output'], currency: 'USD', rate: '10' },
    ]
    expect(rateCardSummary(card, rates, tokens)).toBe('2 rates in EUR, USD · per 1M tokens')
    expect(rateCardSummary(card, [], tokens)).toBe('No rates yet · per 1M tokens')
    const credit = { creditUnitId: 'c', unitSize: '1' }
    expect(rateCardSummary(credit, [{ key: ['*'], rate: '1' }], tokens, 'credits')).toBe(
      '1 rate in credits · per token'
    )
  })

  it('prefixes a rate with its currency symbol', () => {
    expect(currencySymbol('EUR')).toBe('€')
    expect(currencySymbol('USD')).toBe('$')
  })
})
