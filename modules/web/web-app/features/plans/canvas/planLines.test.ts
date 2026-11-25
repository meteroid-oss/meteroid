import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { PriceComponentSchema } from '@/rpc/api/pricecomponents/v1/models_pb'
import { PriceSchema } from '@/rpc/api/prices/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import { firstInvoice, firstInvoiceLine, gapSentence, sellingGaps } from './planLines'

const { MONTHLY, ANNUAL } = BillingPeriod

const price = (currency: string, cadence: BillingPeriod, pricing: object, tag = '') =>
  create(PriceSchema, {
    id: `${currency}-${cadence}${tag}`,
    currency,
    cadence,
    pricing: pricing as never,
  })
const rate = (currency: string, cadence: BillingPeriod, amount: string) =>
  price(currency, cadence, { case: 'ratePricing', value: { rate: amount } }, `-${amount}`)
const usage = (currency: string) =>
  price(currency, MONTHLY, { case: 'usagePricing', value: { model: { case: 'perUnit', value: '3' } } }, '-usage')
const seats = (currency: string, minSlots?: number) =>
  price(currency, MONTHLY, { case: 'slotPricing', value: { unitRate: '12', minSlots } }, '-seat')
const setup = (currency: string) =>
  price(currency, MONTHLY, { case: 'oneTimePricing', value: { unitPrice: '5000', quantity: 1 } }, '-setup')
type P = ReturnType<typeof price>

const standard = { id: 'opt-s', name: 'Standard', alias: 'standard' }
const monthly = { id: 'opt-m', name: 'Monthly', alias: 'monthly' }
const annual = { id: 'opt-y', name: 'Annual', alias: 'annual' }

/** A charge whose prices each option links: `links[option id]` lists its prices. */
const charge = (name: string, links: Record<string, P[]>) => {
  const prices = [...new Map(Object.values(links).flat().map(p => [p.id, p])).values()]
  const optionPrices = Object.entries(links).flatMap(([optionId, linked]) =>
    linked.map(p => ({ optionId, priceId: p.id }))
  )
  return create(PriceComponentSchema, { id: name, name, prices, optionPrices } as never)
}
const noContext = () => ({})

const enterprise = [
  charge('Setup', { [standard.id]: [setup('USD'), setup('EUR')] }),
  charge('Platform fee', { [standard.id]: [rate('USD', ANNUAL, '12000'), rate('EUR', ANNUAL, '11000')] }),
  charge('Input tokens', { [standard.id]: [usage('USD'), usage('EUR')] }),
]
const selfServe = [
  charge('Platform fee', {
    [monthly.id]: [rate('USD', MONTHLY, '99'), rate('EUR', MONTHLY, '89')],
    [annual.id]: [rate('USD', ANNUAL, '950'), rate('EUR', ANNUAL, '850')],
  }),
  charge('Tokens', { [monthly.id]: [usage('USD')], [annual.id]: [usage('USD')] }),
]

describe('firstInvoiceLine', () => {
  it('one-time and upfront fees at start, usage later', () => {
    expect(firstInvoiceLine(enterprise, standard.id, 'USD', noContext)).toBe(
      'First invoice: $17,000 at start · usage invoiced monthly'
    )
  })

  it('one per option, from its own cells', () => {
    expect(firstInvoice(selfServe, monthly.id, 'USD', noContext)).toEqual({
      head: '$99 at start',
      later: 'usage invoiced monthly',
    })
    expect(firstInvoice(selfServe, annual.id, 'USD', noContext)?.head).toBe('$950 at start')
  })

  it('a usage charge lacking a price leaves it as is', () => {
    expect(firstInvoiceLine(selfServe, annual.id, 'EUR', noContext)).toBe(
      'First invoice: €850 at start'
    )
  })

  it('seats without a minimum are what it cannot know', () => {
    const plan = [
      charge('Platform fee', { [standard.id]: [rate('USD', MONTHLY, '99')] }),
      charge('Seats', { [standard.id]: [seats('USD')] }),
    ]
    expect(firstInvoiceLine(plan, standard.id, 'USD', noContext)).toBe(
      'First invoice: $99 + seats at start'
    )
    const withMinimum = [charge('Seats', { [standard.id]: [seats('USD', 3)] })]
    expect(firstInvoiceLine(withMinimum, standard.id, 'USD', noContext)).toBe(
      'First invoice: $36 at start'
    )
  })

  it('none while an upfront fee has no price for the option in the currency', () => {
    const plan = [
      charge('Platform fee', { [monthly.id]: [rate('USD', MONTHLY, '99'), rate('EUR', MONTHLY, '89')] }),
    ]
    expect(firstInvoiceLine(plan, annual.id, 'EUR', noContext)).toBeUndefined()
    expect(firstInvoiceLine(plan, monthly.id, 'GBP', noContext)).toBeUndefined()
  })

  it('a usage-only plan bills nothing at start', () => {
    const plan = [charge('Tokens', { [standard.id]: [usage('USD')] })]
    expect(firstInvoiceLine(plan, standard.id, 'USD', noContext)).toBe(
      'First invoice: nothing at start · usage invoiced monthly'
    )
  })
})

describe('sellingGaps', () => {
  const said = (gaps: ReturnType<typeof sellingGaps>) => gaps.map(gap => gapSentence(gap, 'Pro'))

  it('counts every priced charge, usage included', () => {
    expect(said(sellingGaps(enterprise.slice(0, 2), [standard], ['USD', 'GBP']))).toEqual([
      "GBP has no price for Setup and Platform fee: Pro won't be sold in GBP until each has one.",
    ])
    expect(sellingGaps(enterprise, [standard], ['USD', 'EUR'])).toEqual([])
  })

  it('the same gap in every option is said once, without the options', () => {
    expect(said(sellingGaps(selfServe, [monthly, annual], ['USD', 'EUR']))).toEqual([
      "EUR has no price for Tokens: Pro won't be sold in EUR until it has one.",
    ])
  })

  it('names the option lacking a price', () => {
    const plan = [
      charge('Platform fee', {
        [monthly.id]: [rate('USD', MONTHLY, '99'), rate('EUR', MONTHLY, '89')],
        [annual.id]: [rate('USD', ANNUAL, '950')],
      }),
      charge('Seats', { [monthly.id]: [seats('USD'), seats('EUR')], [annual.id]: [seats('USD')] }),
    ]
    expect(said(sellingGaps(plan, [monthly, annual], ['USD', 'EUR']))).toEqual([
      "EUR, Annual has no price for Platform fee and Seats: Pro won't be sold as Annual in EUR until each has one.",
    ])
  })

  it('a charge included without a price never lacks one', () => {
    const plan = [charge('SSO', {}), charge('Platform fee', { [standard.id]: [rate('USD', MONTHLY, '9')] })]
    expect(sellingGaps(plan, [standard], ['USD'])).toEqual([])
  })
})
