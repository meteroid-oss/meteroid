import { describe, expect, it } from 'vitest'

import {
  cardRatesLabel,
  cardRatesLabelOfMultiplier,
  markupToMultiplier,
  multiplierToMarkup,
} from './markup'

describe('rate card markup', () => {
  it('stores a markup in percent as a multiplier', () => {
    expect(markupToMultiplier('20')).toBe('1.2')
    expect(markupToMultiplier('-20')).toBe('0.8')
    expect(markupToMultiplier('7')).toBe('1.07')
    expect(markupToMultiplier('12.5')).toBe('1.125')
    expect(markupToMultiplier('-100')).toBe('0')
    expect(markupToMultiplier('')).toBeUndefined()
    expect(markupToMultiplier('0')).toBeUndefined()
  })

  it('reads a multiplier back as a markup', () => {
    expect(multiplierToMarkup('1.2')).toBe('20')
    expect(multiplierToMarkup('0.8')).toBe('-20')
    expect(multiplierToMarkup('1.125')).toBe('12.5')
    expect(multiplierToMarkup('2')).toBe('100')
    expect(multiplierToMarkup('1')).toBe('')
    expect(multiplierToMarkup(undefined)).toBe('')
  })

  it('labels the card rates with their markup', () => {
    expect(cardRatesLabel('')).toBe('card rates')
    expect(cardRatesLabel('20')).toBe('card rates +20%')
    expect(cardRatesLabel('-20')).toBe('card rates −20%')
    expect(cardRatesLabelOfMultiplier('0.75')).toBe('card rates −25%')
  })
})
