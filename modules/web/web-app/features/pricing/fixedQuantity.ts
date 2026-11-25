import Decimal from 'decimal.js'

import type { UsagePricing } from '@/rpc/api/prices/v1/models_pb'

/**
 * What a fixed quantity costs: `unitPrice × quantity`, or the quantity priced through its model
 * (per unit, graduated tiers, volume, package), as the billing engine prices it.
 */
export const fixedQuantityTotal = (
  quantity: number,
  unitPrice: string,
  model?: UsagePricing
): Decimal => {
  const q = new Decimal(quantity)
  const m = model?.model
  switch (m?.case) {
    case 'perUnit':
      return q.mul(m.value || 0)
    case 'package': {
      const block = Number(m.value.blockSize) || 1
      return new Decimal(Math.ceil(quantity / block)).mul(m.value.packagePrice || 0)
    }
    case 'tiered':
    case 'volume': {
      // Mirrors billing-core: tiers sorted by their first unit; a graduated tier holds the units up
      // to the next tier's start; a volume bracket holds [first unit, next start).
      const rows = [...m.value.rows].sort((a, b) => Number(a.firstUnit) - Number(b.firstUnit))
      const capped = (amount: Decimal, cap?: string) =>
        cap && amount.greaterThan(cap) ? new Decimal(cap) : amount
      if (m.case === 'volume') {
        const row = rows.find(
          (r, i) =>
            quantity >= Number(r.firstUnit) &&
            (i + 1 === rows.length || quantity < Number(rows[i + 1].firstUnit))
        )
        if (!row) return new Decimal(0)
        return capped(q.mul(row.unitPrice || 0).add(row.flatFee || 0), row.flatCap)
      }
      let remaining = quantity
      let total = new Decimal(0)
      for (const [i, row] of rows.entries()) {
        if (remaining <= 0) break
        const length =
          i + 1 < rows.length
            ? Math.max(0, Number(rows[i + 1].firstUnit) - Number(row.firstUnit))
            : Infinity
        const units = Math.min(remaining, length)
        remaining -= units
        if (units > 0) {
          const amount = new Decimal(units).mul(row.unitPrice || 0).add(row.flatFee || 0)
          total = total.add(capped(amount, row.flatCap))
        }
      }
      return total
    }
    default:
      return q.mul(unitPrice || 0)
  }
}

export const FIXED_MODEL_NAMES: Record<string, string> = {
  perUnit: 'per unit',
  tiered: 'tiered',
  volume: 'volume',
  package: 'package',
}
