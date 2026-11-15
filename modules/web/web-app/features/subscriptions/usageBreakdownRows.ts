import { lineShownBreakdown } from '@/features/invoices/invoiceSummary'

import type { LineBreakdownItem } from '@/features/invoices/invoiceSummary'
import type { LineItem } from '@/rpc/api/invoices/v1/models_pb'

/**
 * A usage line's breakdown as the next invoice shows it. A credit-priced line is shown in
 * credits, so each value shows its part of the credits consumed rather than of the subtotal.
 */
export const usageBreakdownRows = (
  line: Pick<LineItem, 'usageBreakdown' | 'taxInclusiveQuote' | 'creditUnit'>
): { key: string; items: LineBreakdownItem[] } | undefined => {
  const breakdown = lineShownBreakdown(line)
  if (!breakdown || !line.creditUnit || !line.usageBreakdown) return breakdown
  const credits = line.usageBreakdown.items
  return {
    key: breakdown.key,
    items: breakdown.items.map((item, i) => ({
      ...item,
      amount: credits[i].creditsConsumed ?? 0n,
    })),
  }
}
