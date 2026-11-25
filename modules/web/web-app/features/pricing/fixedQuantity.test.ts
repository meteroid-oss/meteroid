import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { UsagePricingSchema } from '@/rpc/api/prices/v1/models_pb'

import { fixedQuantityTotal } from './fixedQuantity'

const tiers = (kind: 'tiered' | 'volume') =>
  create(UsagePricingSchema, {
    model: {
      case: kind,
      value: {
        rows: [
          { firstUnit: 0n, unitPrice: '10' },
          { firstUnit: 5n, unitPrice: '8', flatFee: '2' },
        ],
      },
    },
  })

describe('fixedQuantityTotal', () => {
  it('is unit price × quantity without a model', () => {
    expect(fixedQuantityTotal(3, '12.5').toString()).toBe('37.5')
  })

  it('prices graduated tiers by each tier length', () => {
    // 5 × 10 + (2 × 8 + 2)
    expect(fixedQuantityTotal(7, '0', tiers('tiered')).toString()).toBe('68')
  })

  it('prices every unit at the volume bracket reached', () => {
    expect(fixedQuantityTotal(7, '0', tiers('volume')).toString()).toBe('58')
    expect(fixedQuantityTotal(4, '0', tiers('volume')).toString()).toBe('40')
  })

  it('bills whole packages', () => {
    const pkg = create(UsagePricingSchema, {
      model: { case: 'package', value: { packagePrice: '30', blockSize: 10n } },
    })
    expect(fixedQuantityTotal(21, '0', pkg).toString()).toBe('90')
  })
})
