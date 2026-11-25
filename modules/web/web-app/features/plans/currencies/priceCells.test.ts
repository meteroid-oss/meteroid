import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { PriceComponentSchema } from '@/rpc/api/pricecomponents/v1/models_pb'
import { PriceSchema } from '@/rpc/api/prices/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import {
  optionCell,
  orderCurrencies,
  pricedCurrencies,
  pricedOptionLabel,
  resolveCell,
  sameInEveryOption,
  singleCadence,
  soldOptions,
  tabWarns,
  unpricedIn,
} from './priceCells'

const { MONTHLY, QUARTERLY, ANNUAL } = BillingPeriod

const rate = (currency: string, cadence: BillingPeriod, amount: string) =>
  create(PriceSchema, {
    id: `${currency}-${cadence}-${amount}`,
    cadence,
    currency,
    pricing: { case: 'ratePricing', value: { rate: amount } },
  })
const usage = (currency: string, amount: string) =>
  create(PriceSchema, {
    id: `${currency}-usage`,
    cadence: MONTHLY,
    currency,
    pricing: { case: 'usagePricing', value: { model: { case: 'perUnit', value: amount } } },
  })
type P = ReturnType<typeof rate>

const monthly = { id: 'opt-m', name: 'Monthly', alias: 'monthly' }
const annual = { id: 'opt-y', name: 'Annual', alias: 'annual' }
const options = [monthly, annual]

/** A charge whose prices each option links: `links[option id]` lists its prices. */
const charge = (id: string, links: Record<string, P[]>) => {
  const prices = [...new Map(Object.values(links).flat().map(p => [p.id, p])).values()]
  const optionPrices = Object.entries(links).flatMap(([optionId, linked]) =>
    linked.map(p => ({ optionId, priceId: p.id }))
  )
  return create(PriceComponentSchema, { id, name: id, prices, optionPrices } as never)
}
/** Linked the same in every option. */
const shared = (id: string, prices: P[]) => charge(id, { [monthly.id]: prices, [annual.id]: prices })

describe('optionCell', () => {
  const platform = charge('platform', {
    [monthly.id]: [rate('USD', MONTHLY, '99'), rate('EUR', MONTHLY, '89')],
    [annual.id]: [rate('USD', ANNUAL, '950')],
  })

  it('reads the prices an option links in a currency', () => {
    const cell = optionCell(platform, annual.id, 'USD')
    expect(cell).toHaveLength(1)
    expect(cell[0].pricing.value).toMatchObject({ rate: '950' })
    expect(optionCell(platform, monthly.id, 'EUR')).toHaveLength(1)
  })

  it('an option without a link there has an empty cell, whatever the other options price', () => {
    expect(optionCell(platform, annual.id, 'EUR')).toEqual([])
  })
})

describe('same in every option', () => {
  it('every option links the same price', () => {
    const tokens = shared('tokens', [usage('USD', '3')])
    expect(sameInEveryOption(tokens, options, 'USD')).toBe(true)
  })

  it('a price per option is not', () => {
    const platform = charge('platform', {
      [monthly.id]: [rate('USD', MONTHLY, '99')],
      [annual.id]: [rate('USD', ANNUAL, '950')],
    })
    expect(sameInEveryOption(platform, options, 'USD')).toBe(false)
    expect(sameInEveryOption(platform, options, 'EUR')).toBe(false)
  })

  it('a currency not priced yet merges when every option shares the prices elsewhere', () => {
    expect(sameInEveryOption(shared('tokens', [usage('USD', '3')]), options, 'EUR')).toBe(true)
    expect(sameInEveryOption(charge('sso', {}), options, 'EUR')).toBe(false)
  })

  it('one option never merges', () => {
    expect(sameInEveryOption(shared('tokens', [usage('USD', '3')]), [monthly], 'USD')).toBe(false)
  })
})

describe('where an option is sold', () => {
  const platform = charge('platform', {
    [monthly.id]: [rate('USD', MONTHLY, '99'), rate('EUR', MONTHLY, '89')],
    [annual.id]: [rate('USD', ANNUAL, '950')],
  })
  const tokens = shared('tokens', [usage('USD', '3')])
  const sso = charge('sso', {})
  const components = [platform, tokens, sso]

  it('every priced charge counts, usage included', () => {
    expect(unpricedIn(components, monthly.id, 'EUR').map(c => c.id)).toEqual(['tokens'])
    expect(unpricedIn(components, annual.id, 'EUR').map(c => c.id)).toEqual(['platform', 'tokens'])
    expect(unpricedIn(components, annual.id, 'USD')).toEqual([])
  })

  it('a charge included without a price never lacks one', () => {
    expect(unpricedIn([sso], monthly.id, 'EUR')).toEqual([])
  })

  it('the options sold in a currency', () => {
    expect(soldOptions(options, components, 'USD')).toEqual(options)
    expect(soldOptions(options, [platform], 'EUR')).toEqual([monthly])
  })

  it('warns on a tab where an option lacks a price', () => {
    expect(tabWarns(components, options, 'EUR')).toBe(true)
    expect(tabWarns(components, options, 'USD')).toBe(false)
  })
})

describe('currencies of a version', () => {
  const components = [
    shared('platform', [rate('EUR', ANNUAL, '1390'), rate('USD', MONTHLY, '99')]),
    shared('events', [usage('BRL', '1')]),
  ]

  it('the currencies priced in, the default leading', () => {
    expect(orderCurrencies([...pricedCurrencies(components), 'USD'], 'USD')).toEqual([
      'USD',
      'BRL',
      'EUR',
    ])
  })
})

describe('pricedOptionLabel', () => {
  it('reads a currency and the option sold in it', () => {
    expect(pricedOptionLabel({ currency: 'USD', optionId: annual.id }, options)).toBe('USD · Annual')
  })

  it('a plan with one option reads its currency', () => {
    expect(pricedOptionLabel({ currency: 'BRL', optionId: monthly.id }, [monthly])).toBe('BRL')
  })
})

// Add-ons: tenant-level, priced per currency × cadence.
describe('an add-on cell', () => {
  const addOn = create(PriceComponentSchema, {
    id: 'support',
    name: 'support',
    prices: [rate('USD', MONTHLY, '149'), rate('USD', ANNUAL, '1490'), rate('EUR', MONTHLY, '139')],
  })

  it('reads the price of the currency and cadence', () => {
    const cell = resolveCell(addOn, 'USD', ANNUAL)
    expect(cell.kind === 'own' && cell.prices[0].pricing.value).toMatchObject({ rate: '1490' })
    expect(resolveCell(addOn, 'EUR', ANNUAL).kind).toBe('missing')
  })

  it('one priced on a single cadence bills on it whatever the period', () => {
    const yearly = create(PriceComponentSchema, { prices: [rate('USD', ANNUAL, '1200')] })
    expect(singleCadence(yearly, 'USD')).toBe(ANNUAL)
    expect(resolveCell(yearly, 'USD', MONTHLY).kind).toBe('own')
    expect(resolveCell(yearly, 'USD', QUARTERLY, { exact: true }).kind).toBe('missing')
  })
})
