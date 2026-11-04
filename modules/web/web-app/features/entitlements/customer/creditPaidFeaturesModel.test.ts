import { create } from '@bufbuild/protobuf'
import { timestampFromDate } from '@bufbuild/protobuf/wkt'
import { describe, expect, it } from 'vitest'

import { LineItemSchema } from '@/rpc/api/invoices/v1/models_pb'
import { CreditExhaustedRule_Kind } from '@/rpc/api/plans/v1/models_pb'
import {
  CreditGrantKind,
  CreditGrantSchema,
  CreditGrantSource,
  CreditGrantStatus,
  CreditGroupSchema,
  UsageAtZero,
  WalletPlanRuleSchema,
} from '@/rpc/api/wallets/v1/models_pb'

import {
  creditExhaustedRule,
  creditSources,
  featurePeriodUsage,
  payingGroups,
  refreshesAt,
  sourcesSummary,
  usageOfCredit,
} from './creditPaidFeaturesModel'

const halyard = {
  currency: 'HALYARD_CREDITS',
  creditUnit: { name: 'Halyard credit', pluralName: 'Halyard credits', precision: 0 },
}

const grant = (id: string, source: CreditGrantSource, remaining: bigint) =>
  create(CreditGrantSchema, {
    id,
    source,
    remaining,
    status: CreditGrantStatus.ACTIVE,
    subscriptionPlanName: source === CreditGrantSource.PURCHASE ? undefined : 'Studio',
  })

// Lumen Notes on Studio: a 5,000 pack and 620 rolled over, plus 2,000 bought.
const included = create(CreditGroupSchema, {
  kind: CreditGrantKind.INCLUDED,
  subscriptionId: 'sub_studio',
  label: 'Included with Studio',
  available: 5_620n,
  creditGrantIds: ['pack', 'rolled'],
  nextLapseAt: timestampFromDate(new Date('2026-10-28T00:00:00Z')),
})
const prepaid = create(CreditGroupSchema, {
  kind: CreditGrantKind.PREPAID,
  label: 'Purchased credit',
  available: 2_000n,
  creditGrantIds: ['bought'],
})
const grants = [
  grant('pack', CreditGrantSource.INCLUDED, 5_000n),
  grant('rolled', CreditGrantSource.ROLLOVER, 620n),
  grant('bought', CreditGrantSource.PURCHASE, 2_000n),
]

describe('creditExhaustedRule', () => {
  const rule = (subscriptionId: string, kind: CreditExhaustedRule_Kind) =>
    create(WalletPlanRuleSchema, { subscriptionId, rule: { kind } })
  const payers = [{ subscriptionId: 'sub_a' }]

  it("follows the paying subscription's plan unless the wallet overrides it", () => {
    const planRules = [
      rule('sub_a', CreditExhaustedRule_Kind.PAUSE),
      rule('sub_b', CreditExhaustedRule_Kind.INVOICE),
    ]
    expect(creditExhaustedRule({ usageAtZero: UsageAtZero.FOLLOW_PLAN, planRules }, payers)).toBe(
      'pause'
    )
    expect(creditExhaustedRule({ usageAtZero: UsageAtZero.NEVER_PAUSE, planRules }, payers)).toBe(
      'invoice'
    )
    expect(
      creditExhaustedRule({ usageAtZero: UsageAtZero.FOLLOW_PLAN, planRules }, [
        { subscriptionId: 'sub_b' },
      ])
    ).toBe('invoice')
  })

  it('reports a capped plan and an always-pause override', () => {
    const planRules = [rule('sub_a', CreditExhaustedRule_Kind.INVOICE_UP_TO)]
    expect(creditExhaustedRule({ usageAtZero: UsageAtZero.FOLLOW_PLAN, planRules }, payers)).toBe(
      'cap'
    )
    expect(
      creditExhaustedRule({ usageAtZero: UsageAtZero.ALWAYS_PAUSE, planRules: [] }, payers)
    ).toBe('pause')
  })
})

describe('payingGroups', () => {
  it('keeps the credit of the paying subscription and product, and customer-wide credit', () => {
    const other = create(CreditGroupSchema, {
      kind: CreditGrantKind.INCLUDED,
      subscriptionId: 'sub_other',
    })
    const restricted = create(CreditGroupSchema, {
      kind: CreditGrantKind.BONUS,
      productIds: ['prod_b'],
    })
    const groups = [included, prepaid, other, restricted]

    expect(payingGroups(groups, [{ subscriptionId: 'sub_studio', productId: 'prod_a' }])).toEqual([
      included,
      prepaid,
    ])
    expect(payingGroups(groups, [{ subscriptionId: 'sub_studio', productId: 'prod_b' }])).toEqual([
      included,
      prepaid,
      restricted,
    ])
    expect(payingGroups(groups, [{ subscriptionId: 'sub_studio' }])).toEqual([included, prepaid])
  })
})

describe('creditSources', () => {
  it('names the plan pack, its rollover and the purchases apart, each with what it holds', () => {
    const sources = creditSources([included, prepaid], { grants })

    expect(sources).toEqual([
      { label: 'Studio pack', amount: 5_000n },
      { label: 'Rollover', amount: 620n },
      { label: 'Purchased credit', amount: 2_000n },
    ])
    expect(sourcesSummary(sources)).toBe('Studio pack + purchased credit')
  })

  it('reads past two kinds as multiple sources, and nothing for no credit', () => {
    const bonus = create(CreditGroupSchema, { kind: CreditGrantKind.BONUS, available: 100n })
    const sources = creditSources([included, prepaid, bonus], { grants })

    expect(sourcesSummary(sources)).toBe('Multiple sources')
    expect(sourcesSummary([])).toBeUndefined()
  })
})

describe('refreshesAt', () => {
  it('is the reset of the plan credit, none without plan credit', () => {
    expect(refreshesAt([prepaid, included])).toEqual(included.nextLapseAt)
    expect(refreshesAt([prepaid])).toBeUndefined()
  })
})

describe('featurePeriodUsage', () => {
  const line = (metricId: string, consumed: bigint) =>
    create(LineItemSchema, {
      metricId,
      creditUnit: { creditUnitId: 'unit_halyard', consumed },
    })
  const wallet = { creditUnit: { id: 'unit_halyard' } } as Parameters<typeof featurePeriodUsage>[0]

  it("sums the feature's metric over the paying subscriptions' next invoices", () => {
    expect(
      featurePeriodUsage(wallet, 'tokens', [
        { lineItems: [line('tokens', 3_000n), line('images', 500n)] },
        { lineItems: [line('tokens', 900n)] },
      ])
    ).toBe(3_900n)
  })

  it('is unknown until every invoice is loaded', () => {
    expect(featurePeriodUsage(wallet, 'tokens', [{ lineItems: [] }, undefined])).toBeUndefined()
  })
})

describe('usageOfCredit', () => {
  it('names the unit once', () => {
    expect(usageOfCredit(3_900n, 7_620n, halyard)).toBe('3,900 / 7,620 Halyard credits')
  })
})
