import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  creditRuleSummary,
  defaultCreditRule,
  usageDenominations,
} from '@/features/plans/creditRule'
import { CreditUnitSchema } from '@/rpc/api/creditunits/v1/models_pb'
import { CreditExhaustedRuleSchema, CreditExhaustedRule_Kind } from '@/rpc/api/plans/v1/models_pb'
import { PriceComponentSchema } from '@/rpc/api/pricecomponents/v1/models_pb'
import { PriceSchema } from '@/rpc/api/prices/v1/models_pb'

import type { MessageInitShape } from '@bufbuild/protobuf'

type PriceInit = MessageInitShape<typeof PriceSchema>

const component = (price: PriceInit) =>
  create(PriceComponentSchema, { prices: [create(PriceSchema, price)] })

const usage = (creditUnitId?: string) =>
  component({
    pricing: { case: 'usagePricing', value: {} },
    creditUnit: creditUnitId ? { creditUnitId, rate: '0' } : undefined,
  } as PriceInit)

const pack = component({ pricing: { case: 'creditPricing', value: {} } } as PriceInit)
const fee = component({ pricing: { case: 'ratePricing', value: {} } } as PriceInit)
const units = [
  create(CreditUnitSchema, { id: 'cu', code: 'tok', pluralName: 'Tokens', precision: 2 }),
]

describe('defaultCreditRule', () => {
  it('denies access at zero for a plan that grants credit or prices usage in credits, else invoices', () => {
    expect(defaultCreditRule([fee, usage()])).toBe('INVOICE')
    expect(defaultCreditRule([fee, pack])).toBe('PAUSE')
    expect(defaultCreditRule([usage('cu')])).toBe('PAUSE')
  })
})

describe('usageDenominations', () => {
  it('lists the currency and credit units usage is charged in, else the currency', () => {
    expect(usageDenominations([usage(), usage('cu')], 'eur', units).map(d => d.code)).toEqual([
      'EUR',
      'TOK',
    ])
    expect(usageDenominations([usage('cu')], 'EUR', units).map(d => d.code)).toEqual(['TOK'])
    expect(usageDenominations([fee], 'EUR', units).map(d => d.code)).toEqual(['EUR'])
  })
})

describe('creditRuleSummary', () => {
  it('says the default, and caps in their own units', () => {
    expect(creditRuleSummary(undefined, [pack], [])).toBe('Deny access at zero (default)')
    const denominations = usageDenominations([usage('cu')], 'EUR', units)
    const capped = create(CreditExhaustedRuleSchema, {
      kind: CreditExhaustedRule_Kind.INVOICE_UP_TO,
      caps: [{ denomination: 'TOK', amount: 150000n }],
    })
    expect(creditRuleSummary(capped, [], denominations)).toBe(
      'Invoice overage up to 1500 Tokens, then deny access'
    )
  })
})
