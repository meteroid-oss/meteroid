import { describe, expect, it } from 'vitest'

import { formDataToProtoPricing, protoPricingToFormData } from './mapping'

describe('seat pricing', () => {
  it('sends a per-seat rate alone', () => {
    const pricing = formDataToProtoPricing('slot', { seatModel: 'per_unit', unitRate: '10.00' })
    expect(pricing.case).toBe('slotPricing')
    if (pricing.case !== 'slotPricing') return
    expect(pricing.value.unitRate).toBe('10.00')
    expect(pricing.value.model).toBeUndefined()
  })

  it('round-trips graduated tiers on the seat count', () => {
    const rows = [
      { firstUnit: BigInt(0), unitPrice: '10.00' },
      { firstUnit: BigInt(10), unitPrice: '8.00' },
    ]
    const pricing = formDataToProtoPricing('slot', { seatModel: 'tiered', rows, minSlots: 1 })
    expect(pricing.case).toBe('slotPricing')
    if (pricing.case !== 'slotPricing') return
    expect(pricing.value.unitRate).toBe('')
    expect(pricing.value.model?.model.case).toBe('tiered')

    const form = protoPricingToFormData(pricing)
    expect(form?.pricingType).toBe('slot')
    expect(form?.data.seatModel).toBe('tiered')
    expect(form?.data.minSlots).toBe(1)
    expect((form?.data.rows as unknown[]).length).toBe(2)
  })
})
