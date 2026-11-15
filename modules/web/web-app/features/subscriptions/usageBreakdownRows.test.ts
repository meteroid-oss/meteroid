import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  LineItemSchema,
  LineItem_CreditUnitUsageSchema,
  LineItem_UsageBreakdownSchema,
  LineItem_UsageBreakdown_ItemSchema,
} from '@/rpc/api/invoices/v1/models_pb'

import { usageBreakdownRows } from './usageBreakdownRows'

const breakdown = create(LineItem_UsageBreakdownSchema, {
  key: 'project_id',
  items: [
    create(LineItem_UsageBreakdown_ItemSchema, {
      value: 'search',
      quantity: '600',
      subtotal: 6000n,
      creditsConsumed: 60n,
    }),
    create(LineItem_UsageBreakdown_ItemSchema, { quantity: '300', subtotal: 3000n }),
    create(LineItem_UsageBreakdown_ItemSchema, {
      others: true,
      quantity: '100',
      subtotal: 1000n,
      creditsConsumed: 10n,
    }),
  ],
})

describe('usageBreakdownRows', () => {
  it('is absent for a line without a breakdown', () => {
    expect(usageBreakdownRows(create(LineItemSchema, {}))).toBeUndefined()
  })

  it('lists each value with its part of the subtotal', () => {
    const rows = usageBreakdownRows(create(LineItemSchema, { usageBreakdown: breakdown }))
    expect(rows?.key).toBe('project_id')
    expect(rows?.items.map(i => [i.label, i.noValue, i.quantity, i.amount])).toEqual([
      ['search', false, '600', 6000n],
      ['—', true, '300', 3000n],
      ['Other', false, '100', 1000n],
    ])
  })

  it('shows a credit-priced line in credits', () => {
    const rows = usageBreakdownRows(
      create(LineItemSchema, {
        usageBreakdown: breakdown,
        creditUnit: create(LineItem_CreditUnitUsageSchema, {}),
      })
    )
    expect(rows?.items.map(i => i.amount)).toEqual([60n, 0n, 10n])
  })
})
