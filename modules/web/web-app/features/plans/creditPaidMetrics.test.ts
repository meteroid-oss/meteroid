import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { PriceComponentSchema } from '@/rpc/api/pricecomponents/v1/models_pb'
import { PriceSchema } from '@/rpc/api/prices/v1/models_pb'
import { ProductSchema } from '@/rpc/api/products/v1/models_pb'

import { creditPaidLabels } from './creditPaidMetrics'

const usage = (metricId: string) =>
  create(ProductSchema, {
    feeStructure: { structure: { case: 'usage', value: { metricId } } },
  })

describe('creditPaidLabels', () => {
  it('labels the metrics priced in a credit unit only', () => {
    const labels = creditPaidLabels(
      [
        create(PriceComponentSchema, {
          productId: 'prod_tokens',
          prices: [
            create(PriceSchema, { creditUnit: { creditUnitId: 'unit_halyard', rate: '0.01' } }),
          ],
        }),
        create(PriceComponentSchema, {
          productId: 'prod_storage',
          prices: [create(PriceSchema, {})],
        }),
      ],
      new Map([
        ['prod_tokens', usage('tokens')],
        ['prod_storage', usage('storage')],
      ]),
      [{ id: 'unit_halyard', pluralName: 'Halyard credits' }]
    )

    expect([...labels]).toEqual([['tokens', 'Paid with credit (Halyard credits)']])
  })
})
