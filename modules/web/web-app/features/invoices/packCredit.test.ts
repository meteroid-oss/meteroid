import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { packCreditText } from '@/features/invoices/packCredit'
import {
  LineItemKind,
  LineItemSchema,
  LineItem_CreditGrantUnitSchema,
} from '@/rpc/api/invoices/v1/models_pb'

describe('packCreditText', () => {
  it('names the credits a unit pack grants', () => {
    const line = create(LineItemSchema, {
      kind: LineItemKind.CREDIT,
      creditGrantAmount: 50_000n,
      creditGrantUnit: create(LineItem_CreditGrantUnitSchema, {
        creditUnitId: 'cu_1',
        code: 'FAST',
        precision: 2,
      }),
    })
    const units = { cu_1: { name: 'fast request', pluralName: 'fast requests', precision: 2 } }
    expect(packCreditText(line, 'EUR', units)).toMatch(/^Adds 500 fast requests$/)
    expect(packCreditText(line, 'EUR', {})).toMatch(/^Adds 500 FAST$/)
    const reversed = { ...line, creditGrantAmount: -20_000n }
    expect(packCreditText(reversed, 'EUR', units, 'Reverses')).toMatch(
      /^Reverses 200 fast requests$/
    )
  })

  // The ledger mints a money pack paid below its credit as paid credit worth what was paid, plus
  // bonus credit for the rest, whether the pack is priced below its credit or discounted.
  it('splits a money pack paid below its credit into paid and bonus credit, as the ledger', () => {
    const below = create(LineItemSchema, {
      kind: LineItemKind.CREDIT,
      creditGrantAmount: 5_000n,
      subtotal: 4_000n,
      taxableAmount: 4_000n,
    })
    expect(packCreditText(below, 'EUR', {})).toBe(
      'Adds €50.00 of credit · €40.00 paid + €10.00 bonus credit'
    )
    // Priced at its credit, 20% off by a coupon.
    const coupon = create(LineItemSchema, {
      kind: LineItemKind.CREDIT,
      creditGrantAmount: 5_000n,
      subtotal: 5_000n,
      taxableAmount: 4_000n,
    })
    expect(packCreditText(coupon, 'EUR', {})).toBe(
      'Adds €50.00 of credit · €40.00 paid + €10.00 bonus credit'
    )
    // Priced at €40 for €50 of credit, 20% off.
    const both = create(LineItemSchema, {
      kind: LineItemKind.CREDIT,
      creditGrantAmount: 5_000n,
      subtotal: 4_000n,
      taxableAmount: 3_200n,
    })
    expect(packCreditText(both, 'EUR', {})).toBe(
      'Adds €50.00 of credit · €32.00 paid + €18.00 bonus credit'
    )
    const atFace = create(LineItemSchema, {
      kind: LineItemKind.CREDIT,
      creditGrantAmount: 5_000n,
      subtotal: 5_000n,
      taxableAmount: 5_000n,
    })
    expect(packCreditText(atFace, 'EUR', {})).toBe('Adds €50.00 of credit')
    expect(packCreditText(create(LineItemSchema, { kind: LineItemKind.REGULAR }), 'EUR', {})).toBe(
      undefined
    )
  })
})
