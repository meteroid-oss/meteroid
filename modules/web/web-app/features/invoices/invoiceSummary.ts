import type { DetailedInvoice, LineItem } from '@/rpc/api/invoices/v1/models_pb'

export interface InvoiceSummaryRow {
  label: string
  amount: bigint
}

/**
 * The figures between subtotal and total, as the PDF states them (`typst_render.rs`). A
 * tax-inclusive invoice is shown in its quoted, tax-included figures: the subtotal is what the
 * lines were quoted, the reductions what was taken off those prices, and the tax is included in
 * the total, with the net total on its own row. Otherwise every figure is net.
 */
export interface InvoiceSummary {
  subtotal: bigint
  reductions: InvoiceSummaryRow[]
  /** Tax-inclusive only: the total before the tax carved out of it. */
  totalExclTax?: bigint
  taxIncluded: boolean
}

type SummaryInvoice = Pick<
  DetailedInvoice,
  | 'taxInclusive'
  | 'lineItems'
  | 'subtotal'
  | 'couponLineItems'
  | 'discount'
  | 'promotionalCredits'
  | 'quotedPromotionalCredits'
  | 'total'
  | 'taxAmount'
>

/** What a line shows: its quoted figure on a tax-inclusive invoice. */
export const lineShownSubtotal = (line: Pick<LineItem, 'subtotal' | 'taxInclusiveQuote'>) =>
  line.taxInclusiveQuote?.subtotal ?? line.subtotal

export const lineShownUnitPrice = (line: Pick<LineItem, 'unitPrice' | 'taxInclusiveQuote'>) =>
  line.taxInclusiveQuote ? line.taxInclusiveQuote.unitPrice : line.unitPrice

export const lineShownSubLines = (line: Pick<LineItem, 'subLineItems' | 'taxInclusiveQuote'>) =>
  line.taxInclusiveQuote?.subLineItems.length
    ? line.taxInclusiveQuote.subLineItems
    : line.subLineItems

export const invoiceSummary = (invoice: SummaryInvoice): InvoiceSummary => {
  const reductions: InvoiceSummaryRow[] = []
  if (!invoice.taxInclusive) {
    if (invoice.couponLineItems.length > 0) {
      invoice.couponLineItems.forEach(c => reductions.push({ label: c.name, amount: c.total }))
    } else if (invoice.discount > 0n) {
      reductions.push({ label: 'Discount', amount: invoice.discount })
    }
    if (invoice.promotionalCredits > 0n) {
      reductions.push({ label: 'Free credit', amount: invoice.promotionalCredits })
    }
    return { subtotal: invoice.subtotal, reductions, taxIncluded: false }
  }

  const subtotal = invoice.lineItems.reduce((sum, line) => sum + lineShownSubtotal(line), 0n)
  const promotional = invoice.quotedPromotionalCredits ?? invoice.promotionalCredits
  if (invoice.couponLineItems.length > 0) {
    invoice.couponLineItems.forEach(c =>
      reductions.push({ label: c.name, amount: c.quotedTotal ?? c.total })
    )
  } else {
    const discount = subtotal - invoice.total - promotional
    if (discount > 0n) reductions.push({ label: 'Discount', amount: discount })
  }
  if (promotional > 0n) reductions.push({ label: 'Free credit', amount: promotional })
  return {
    subtotal,
    reductions,
    totalExclTax: invoice.total - invoice.taxAmount,
    taxIncluded: true,
  }
}
