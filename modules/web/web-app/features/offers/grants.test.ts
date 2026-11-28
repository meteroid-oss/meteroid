import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { formatCurrencyNoRounding } from '@/lib/utils/numbers'
import { EntitlementEffect } from '@/rpc/api/entitlements/v1/models_pb'
import {
  GrantAddOn_AtEnd,
  GrantCredit_Scope,
  GrantSchema,
  GrantWindowSchema,
  GrantWindow_Anchor,
  Grant_Kind,
} from '@/rpc/api/offers/v1/offers_pb'

import {
  grantChip,
  grantDefinitionLabel,
  grantFromProto,
  grantKinds,
  grantToProto,
  shortWindowLabel,
  windowFromProto,
  windowIssue,
  windowLabel,
  windowToProto,
} from './grants'

import type { GrantForm, WindowForm } from './grants'

const asStored = (grant: GrantForm, kind: Grant_Kind) => {
  const created = grantToProto(grant)
  return create(GrantSchema, {
    id: grant.key,
    kind,
    window: created.window,
    definition: created.definition as never,
  })
}

describe('windowLabel', () => {
  it('reads a window relative to its anchor', () => {
    expect(windowLabel({ anchor: 'start', duration: 'days', count: 30 })).toBe('30 days from start')
    expect(windowLabel({ anchor: 'billing', duration: 'months', count: 3 })).toBe(
      '3 months from billing'
    )
    expect(windowLabel({ anchor: 'start', duration: 'months', count: 1 })).toBe('1 month from start')
    expect(windowLabel({ anchor: 'start', duration: 'untilBilling' })).toBe('until the trial ends')
    expect(windowLabel({ anchor: 'start', duration: 'forever' })).toBe('forever')
    expect(windowLabel({ anchor: 'billing', duration: 'invoices', count: 3 })).toBe(
      'first 3 invoices'
    )
  })

  it('shortens it for chips', () => {
    expect(shortWindowLabel({ anchor: 'start', duration: 'days', count: 30 })).toBe('30 d')
    expect(shortWindowLabel({ anchor: 'billing', duration: 'months', count: 3 })).toBe('3 months')
    expect(shortWindowLabel({ anchor: 'start', duration: 'untilBilling' })).toBe('trial')
    expect(shortWindowLabel({ anchor: 'start', duration: 'forever' })).toBe('forever')
  })
})

describe('windowIssue', () => {
  it('keeps each kind to the windows its engine supports', () => {
    expect(windowIssue('discount', { anchor: 'start', duration: 'months', count: 3 })).toBeDefined()
    expect(windowIssue('discount', { anchor: 'billing', duration: 'days', count: 3 })).toBeDefined()
    expect(windowIssue('discount', { anchor: 'billing', duration: 'months', count: 3 })).toBeUndefined()
    expect(windowIssue('credit', { anchor: 'start', duration: 'untilBilling' })).toBeDefined()
    expect(windowIssue('addOn', { anchor: 'start', duration: 'days', count: 30 })).toBeDefined()
    expect(windowIssue('entitlement', { anchor: 'start', duration: 'untilBilling' })).toBeUndefined()
  })

  it('counts a trial-bound window from the start, and needs a count where it has one', () => {
    expect(windowIssue('entitlement', { anchor: 'billing', duration: 'untilBilling' })).toBeDefined()
    expect(windowIssue('credit', { anchor: 'start', duration: 'days', count: 0 })).toBeDefined()
    expect(windowIssue('credit', { anchor: 'start', duration: 'days', count: 1.5 })).toBeDefined()
    expect(windowIssue('credit', { anchor: 'start', duration: 'months', count: 121 })).toBeDefined()
  })
})

describe('window ↔ proto', () => {
  const windows: WindowForm[] = [
    { anchor: 'start', duration: 'days', count: 30 },
    { anchor: 'billing', duration: 'months', count: 3 },
    { anchor: 'start', duration: 'untilBilling' },
    { anchor: 'billing', duration: 'forever' },
    { anchor: 'billing', duration: 'invoices', count: 2 },
  ]

  it('round-trips every duration', () => {
    for (const window of windows) {
      expect(windowFromProto(create(GrantWindowSchema, windowToProto(window)))).toEqual(window)
    }
  })

  it('maps the anchor and the duration case', () => {
    expect(windowToProto({ anchor: 'billing', duration: 'untilBilling' })).toEqual({
      anchor: GrantWindow_Anchor.BILLING_START,
      duration: { case: 'untilBillingStarts', value: true },
    })
    expect(windowFromProto(undefined)).toEqual({ anchor: 'start', duration: 'forever' })
  })
})

describe('grant ↔ proto', () => {
  const credit: GrantForm = {
    key: 'g1',
    kind: 'credit',
    window: { anchor: 'start', duration: 'days', count: 30 },
    currency: 'USD',
    amount: ' 50.00 ',
    scope: 'subscription',
  }
  const discount: GrantForm = {
    key: 'g2',
    kind: 'discount',
    window: { anchor: 'billing', duration: 'months', count: 3 },
    discount: {
      type: 'fixed',
      currency: 'EUR',
      amount: '10',
      otherAmounts: [
        { currency: 'USD', amount: '11' },
        { currency: 'EUR', amount: '9' },
        { currency: '', amount: '' },
      ],
    },
    addOnIds: ['add_1'],
  }
  const feature: GrantForm = {
    key: 'g3',
    kind: 'entitlement',
    window: { anchor: 'start', duration: 'untilBilling' },
    featureId: 'feat_sso',
    value: undefined,
    effect: EntitlementEffect.UNSPECIFIED,
  }
  const addOn: GrantForm = {
    key: 'g4',
    kind: 'addOn',
    window: { anchor: 'start', duration: 'months', count: 1 },
    addOnId: 'add_ai',
    quantity: 2,
    atEnd: 'convert',
  }

  it('sends a credit trimmed, with its scope', () => {
    expect(grantToProto(credit).definition).toEqual({
      case: 'credit',
      value: { currency: 'USD', amount: '50.00', scope: GrantCredit_Scope.SUBSCRIPTION },
    })
  })

  it('sends a fixed discount’s other currencies only, without its own or empty rows', () => {
    expect(grantToProto(discount).definition).toEqual({
      case: 'discount',
      value: {
        discount: { case: 'fixed', value: { currency: 'EUR', amount: '10' } },
        fixedAmounts: [{ currency: 'USD', amount: '11' }],
        addOnIds: ['add_1'],
      },
    })
  })

  it('sends an add-on kept at the end as converted', () => {
    expect(grantToProto(addOn).definition).toEqual({
      case: 'addOn',
      value: { addOnId: 'add_ai', quantity: 2, atEnd: GrantAddOn_AtEnd.CONVERT },
    })
  })

  it('reads stored grants back as the editor holds them', () => {
    expect(grantFromProto(asStored(credit, Grant_Kind.CREDIT))).toEqual({
      ...credit,
      amount: '50.00',
    })
    expect(grantFromProto(asStored(addOn, Grant_Kind.ADD_ON))).toEqual(addOn)
    const readFeature = grantFromProto(asStored(feature, Grant_Kind.ENTITLEMENT))
    expect(readFeature).toMatchObject({ kind: 'entitlement', featureId: 'feat_sso', window: feature.window })
    expect(grantFromProto(asStored(discount, Grant_Kind.DISCOUNT))).toEqual({
      ...discount,
      discount: { ...discount.discount, otherAmounts: [{ currency: 'USD', amount: '11' }] },
    })
  })

  it('leaves out a grant without a definition', () => {
    expect(grantFromProto(create(GrantSchema, { id: 'g9' }))).toBeUndefined()
  })
})

describe('grant labels', () => {
  const window: WindowForm = { anchor: 'start', duration: 'days', count: 30 }
  const names = {
    feature: (id: string) => (id === 'feat_sso' ? 'SSO' : undefined),
    addOn: (id: string) => (id === 'add_ai' ? 'AI' : undefined),
  }

  it('says what each kind gives', () => {
    expect(
      grantDefinitionLabel({
        key: 'a',
        kind: 'discount',
        window,
        discount: { type: 'percentage', percentage: '20' },
        addOnIds: [],
      })
    ).toBe('20% off')
    expect(
      grantDefinitionLabel({
        key: 'b',
        kind: 'discount',
        window,
        discount: { type: 'percentage', percentage: '20' },
        addOnIds: ['x', 'y'],
      })
    ).toBe('20% off · 2 add-ons')
    expect(
      grantDefinitionLabel({
        key: 'c',
        kind: 'credit',
        window,
        currency: 'USD',
        amount: '50',
        scope: 'customer',
      })
    ).toBe(`${formatCurrencyNoRounding('50', 'USD')} credit`)
    expect(
      grantDefinitionLabel(
        {
          key: 'd',
          kind: 'entitlement',
          window,
          featureId: 'feat_sso',
          value: undefined,
          effect: EntitlementEffect.UNSPECIFIED,
        },
        names
      )
    ).toBe('SSO · enabled')
    expect(
      grantDefinitionLabel(
        { key: 'e', kind: 'addOn', window, addOnId: 'add_ai', quantity: 2, atEnd: 'convert' },
        names
      )
    ).toBe('AI × 2 · free, then paid')
  })

  it('makes chips of a grant and its window', () => {
    expect(
      grantChip({ key: 'c', kind: 'credit', window, currency: 'USD', amount: '50', scope: 'customer' })
    ).toEqual({
      key: 'c',
      label: `Credit ${formatCurrencyNoRounding('50', 'USD')} · 30 d`,
      tone: 'success',
    })
    expect(
      grantChip(
        {
          key: 'd',
          kind: 'entitlement',
          window,
          featureId: 'feat_sso',
          value: undefined,
          effect: EntitlementEffect.UNSPECIFIED,
        },
        names
      ).label
    ).toBe('SSO · 30 d')
    expect(
      grantChip({
        key: 'e',
        kind: 'addOn',
        window: { anchor: 'start', duration: 'months', count: 1 },
        addOnId: 'unknown',
        quantity: 1,
        atEnd: 'revert',
      }).label
    ).toBe('Add-on · free 1 month')
  })

  it('offers add-ons and features only where allowed', () => {
    expect(grantKinds({ addOns: false, features: true })).toEqual(['credit', 'entitlement', 'discount'])
    expect(grantKinds({ addOns: true, features: false })).toEqual(['credit', 'discount', 'addOn'])
  })
})
