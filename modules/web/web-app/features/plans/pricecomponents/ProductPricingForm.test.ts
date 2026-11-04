import { describe, expect, it } from 'vitest'

import { buildDefaults } from './ProductPricingForm'

describe('buildDefaults for a usage product without a price', () => {
  it('seeds the pricing fields of the product usage model', () => {
    expect(buildDefaults('usage', undefined, false, 'tiered')).toMatchObject({
      usageModel: 'tiered',
      rows: [{ firstUnit: BigInt(0) }, { firstUnit: BigInt(100) }],
    })
    expect(buildDefaults('usage', undefined, false, 'package')).toMatchObject({
      usageModel: 'package',
      blockSize: 1,
    })
    expect(buildDefaults('usage', undefined, false, 'tiered')).not.toHaveProperty('unitPrice')
  })

  it('falls back to per unit', () => {
    expect(buildDefaults('usage')).toMatchObject({
      usageModel: 'per_unit',
      unitPrice: '0.00000000',
    })
  })
})
