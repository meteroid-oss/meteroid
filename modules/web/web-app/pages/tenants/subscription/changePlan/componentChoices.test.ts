import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { PriceComponentSchema } from '@/rpc/api/pricecomponents/v1/models_pb'
import { PriceSchema } from '@/rpc/api/prices/v1/models_pb'
import { BillingPeriod } from '@/rpc/api/shared/v1/shared_pb'
import {
  SubscriptionComponentSchema,
  SubscriptionFeeBillingPeriod,
} from '@/rpc/api/subscriptions/v1/models_pb'

import {
  configurableComponents,
  parametersKey,
  toParameterizedComponents,
} from './componentChoices'

import type { Price } from '@/rpc/api/prices/v1/models_pb'

const rate = (id: string, cadence: BillingPeriod) =>
  create(PriceSchema, { id, cadence, pricing: { case: 'ratePricing', value: { rate: '10' } } })

const capacity = (id: string, cadence: BillingPeriod, included: bigint) =>
  create(PriceSchema, {
    id,
    cadence,
    pricing: { case: 'capacityPricing', value: { rate: '50', included, overageRate: '1' } },
  })

const slot = (id: string, cadence: BillingPeriod, minSlots?: number, maxSlots?: number) =>
  create(PriceSchema, {
    id,
    cadence,
    pricing: { case: 'slotPricing', value: { unitRate: '5', minSlots, maxSlots } },
  })

const component = (id: string, productId: string, prices: Price[]) =>
  create(PriceComponentSchema, { id, name: id, productId, prices })

const held = (productId: string, period: SubscriptionFeeBillingPeriod) =>
  create(SubscriptionComponentSchema, { id: `sc_${productId}`, productId, period })

const params = (
  targets: ReturnType<typeof component>[],
  current: ReturnType<typeof held>[],
  choices = {}
) => toParameterizedComponents(configurableComponents(targets, current, choices))

describe('plan change component parameters', () => {
  const quarterly = [held('prod_base', SubscriptionFeeBillingPeriod.QUARTERLY)]

  it('sends nothing for single-price and matched components', () => {
    const targets = [
      component('pc_rate', 'prod_rate', [rate('r', BillingPeriod.MONTHLY)]),
      component('pc_base', 'prod_base', [
        rate('m', BillingPeriod.MONTHLY),
        rate('a', BillingPeriod.ANNUAL),
      ]),
    ]
    expect(params(targets, quarterly)).toEqual([])
  })

  it("defaults the cadence to the subscription's, else the shortest", () => {
    const both = [
      rate('m', BillingPeriod.MONTHLY),
      rate('q', BillingPeriod.QUARTERLY),
      rate('a', BillingPeriod.ANNUAL),
    ]
    expect(params([component('pc_new', 'prod_new', both)], quarterly)).toEqual([
      { componentId: 'pc_new', billingPeriod: BillingPeriod.QUARTERLY },
    ])
    expect(params([component('pc_new', 'prod_new', [both[2], both[0]])], quarterly)).toEqual([
      { componentId: 'pc_new', billingPeriod: BillingPeriod.MONTHLY },
    ])
  })

  it('defaults to the lowest capacity tier of the chosen cadence and keeps a valid choice', () => {
    const target = component('pc_cap', 'prod_cap', [
      capacity('m500', BillingPeriod.MONTHLY, 500n),
      capacity('m100', BillingPeriod.MONTHLY, 100n),
      capacity('a1000', BillingPeriod.ANNUAL, 1000n),
      capacity('a2000', BillingPeriod.ANNUAL, 2000n),
    ])
    const monthly = [held('prod_base', SubscriptionFeeBillingPeriod.MONTHLY)]

    expect(params([target], monthly)).toEqual([
      { componentId: 'pc_cap', billingPeriod: BillingPeriod.MONTHLY, committedCapacity: 100n },
    ])
    expect(params([target], monthly, { pc_cap: { committedCapacity: 500n } })).toEqual([
      { componentId: 'pc_cap', billingPeriod: BillingPeriod.MONTHLY, committedCapacity: 500n },
    ])
    expect(
      params([target], monthly, {
        pc_cap: { billingPeriod: BillingPeriod.ANNUAL, committedCapacity: 500n },
      })
    ).toEqual([
      { componentId: 'pc_cap', billingPeriod: BillingPeriod.ANNUAL, committedCapacity: 1000n },
    ])
  })

  it('always sends the seats of an added slot component, within its bounds', () => {
    const seats = (minSlots?: number, maxSlots?: number, initialSlotCount?: number) =>
      params(
        [
          component('pc_seats', 'prod_seats', [
            slot('s', BillingPeriod.MONTHLY, minSlots, maxSlots),
          ]),
        ],
        quarterly,
        initialSlotCount === undefined ? {} : { pc_seats: { initialSlotCount } }
      )[0]?.initialSlotCount

    expect(seats()).toBe(1)
    expect(seats(3)).toBe(3)
    expect(seats(3, 10, 7)).toBe(7)
    expect(seats(3, 10, 1)).toBe(3)
    expect(seats(3, 10, 50)).toBe(10)
  })

  it('treats a legacy component as added even when the product is held', () => {
    const legacy = component('pc_legacy', 'prod_base', [
      capacity('', BillingPeriod.MONTHLY, 100n),
      capacity('', BillingPeriod.MONTHLY, 200n),
    ])
    expect(params([legacy], quarterly)).toEqual([
      { componentId: 'pc_legacy', committedCapacity: 100n },
    ])
  })

  it('keys parameters for change detection', () => {
    expect(
      parametersKey([
        { componentId: 'a', billingPeriod: BillingPeriod.ANNUAL, committedCapacity: 10n },
        { componentId: 'b', initialSlotCount: 2 },
      ])
    ).toBe('a:2:10:|b:::2')
  })
})
