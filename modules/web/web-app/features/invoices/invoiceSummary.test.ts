import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { invoiceSummary } from '@/features/invoices/invoiceSummary'
import {
  DetailedInvoiceSchema,
  LineItemSchema,
  LineItem_TaxInclusiveQuoteSchema,
} from '@/rpc/api/invoices/v1/models_pb'

describe('invoiceSummary', () => {
  it('states a tax-inclusive promotional credit as quoted, like the PDF', () => {
    // 120.00 quoted less 50.00 of promotional credit is 70.00, of which 11.67 is tax; the net
    // share of the credit is 41.66 (test_credit_promotional.rs).
    const invoice = create(DetailedInvoiceSchema, {
      taxInclusive: true,
      lineItems: [
        create(LineItemSchema, {
          subtotal: 9_999n,
          taxInclusiveQuote: create(LineItem_TaxInclusiveQuoteSchema, { subtotal: 12_000n }),
        }),
      ],
      subtotal: 9_999n,
      promotionalCredits: 4_166n,
      quotedPromotionalCredits: 5_000n,
      total: 7_000n,
      taxAmount: 1_167n,
    })
    const summary = invoiceSummary(invoice)
    expect(summary.subtotal).toBe(12_000n)
    expect(summary.reductions).toEqual([{ label: 'Free credit', amount: 5_000n }])
    expect(summary.totalExclTax).toBe(5_833n)
    expect(summary.taxIncluded).toBe(true)
    const reduced = summary.reductions.reduce((sum, row) => sum + row.amount, 0n)
    expect(summary.subtotal - reduced).toBe(invoice.total)
  })

  it('keeps net figures on a tax-exclusive invoice', () => {
    const invoice = create(DetailedInvoiceSchema, {
      subtotal: 10_000n,
      discount: 2_000n,
      promotionalCredits: 500n,
      total: 9_000n,
      taxAmount: 1_500n,
    })
    const summary = invoiceSummary(invoice)
    expect(summary.subtotal).toBe(10_000n)
    expect(summary.reductions).toEqual([
      { label: 'Discount', amount: 2_000n },
      { label: 'Free credit', amount: 500n },
    ])
    expect(summary.totalExclTax).toBeUndefined()
    const reduced = summary.reductions.reduce((sum, row) => sum + row.amount, 0n)
    expect(summary.subtotal - reduced + invoice.taxAmount).toBe(invoice.total)
  })
})
