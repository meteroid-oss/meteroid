import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { PriceComponentSchema } from '@/rpc/api/pricecomponents/v1/models_pb'
import { PriceSchema } from '@/rpc/api/prices/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'

import { chargeLines, gapLabel, linesKey } from './usedIn'

const { MONTHLY, ANNUAL } = BillingPeriod

const seats = (currency: string, cadence: BillingPeriod, amount: string) =>
  create(PriceSchema, {
    id: `${currency}-${cadence}-${amount}`,
    currency,
    cadence,
    pricing: { case: 'slotPricing', value: { unitRate: amount } },
  })
type P = ReturnType<typeof seats>

const monthly = { id: 'opt-m', name: 'Monthly' }
const annual = { id: 'opt-y', name: 'Annual' }

/** A charge whose prices each option links: `links[option id]` lists its prices. */
const charge = (links: Record<string, P[]>) => {
  const prices = [
    ...new Map(
      Object.values(links)
        .flat()
        .map(p => [p.id, p])
    ).values(),
  ]
  const optionPrices = Object.entries(links).flatMap(([optionId, linked]) =>
    linked.map(p => ({ optionId, priceId: p.id }))
  )
  return create(PriceComponentSchema, { id: 'seats', name: 'Seats', prices, optionPrices })
}

const amounts = (lines: ReturnType<typeof chargeLines>) =>
  lines.map(line => ({
    currency: line.currency,
    cells: line.cells.map(c => `${c.option ?? '*'}:${c.prices.map(p => p.id).join('+') || '-'}`),
  }))

describe('chargeLines', () => {
  it('shows a cell per option, a gap where an option has no price', () => {
    const usdM = seats('USD', MONTHLY, '12')
    const usdY = seats('USD', ANNUAL, '120')
    const eurM = seats('EUR', MONTHLY, '11')
    const lines = chargeLines(
      charge({ 'opt-m': [usdM, eurM], 'opt-y': [usdY] }),
      [monthly, annual],
      ['USD', 'EUR']
    )
    expect(amounts(lines)).toEqual([
      { currency: 'EUR', cells: [`Monthly:${eurM.id}`, 'Annual:-'] },
      { currency: 'USD', cells: [`Monthly:${usdM.id}`, `Annual:${usdY.id}`] },
    ])
  })

  it('shows one cell when every option shares the price', () => {
    const usd = seats('USD', MONTHLY, '12')
    const lines = chargeLines(
      charge({ 'opt-m': [usd], 'opt-y': [usd] }),
      [monthly, annual],
      ['USD']
    )
    expect(amounts(lines)).toEqual([{ currency: 'USD', cells: [`*:${usd.id}`] }])
  })

  it('shows one gap for a currency of the plan the charge has no price in', () => {
    const usd = seats('USD', MONTHLY, '12')
    const lines = chargeLines(
      charge({ 'opt-m': [usd], 'opt-y': [usd] }),
      [monthly, annual],
      ['EUR', 'USD']
    )
    expect(amounts(lines)[0]).toEqual({ currency: 'EUR', cells: ['*:-'] })
    expect(gapLabel(lines[0].cells[0])).toBe('No price')
  })

  it('reads the single option alone, and a currency only the charge has', () => {
    const usd = seats('USD', MONTHLY, '12')
    const gbp = seats('GBP', MONTHLY, '10')
    const lines = chargeLines(
      charge({ 'opt-s': [usd, gbp] }),
      [{ id: 'opt-s', name: 'Standard' }],
      ['USD']
    )
    expect(amounts(lines)).toEqual([
      { currency: 'GBP', cells: [`*:${gbp.id}`] },
      { currency: 'USD', cells: [`*:${usd.id}`] },
    ])
  })
})

describe('gapLabel', () => {
  it('names the option lacking a price', () => {
    expect(gapLabel({ option: 'Annual', prices: [] })).toBe('Annual: no price')
  })
})

describe('linesKey', () => {
  it('is the same for two charges priced alike, else differs', () => {
    const options = [monthly, annual]
    const a = chargeLines(charge({ 'opt-m': [seats('USD', MONTHLY, '12')] }), options, ['USD'])
    const b = chargeLines(charge({ 'opt-m': [seats('USD', MONTHLY, '12')] }), options, ['USD'])
    const c = chargeLines(charge({ 'opt-m': [seats('USD', MONTHLY, '15')] }), options, ['USD'])
    expect(linesKey(a, {})).toBe(linesKey(b, {}))
    expect(linesKey(a, {})).not.toBe(linesKey(c, {}))
  })
})
