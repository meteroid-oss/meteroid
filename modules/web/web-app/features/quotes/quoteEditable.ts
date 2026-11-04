import { DetailedQuote, QuoteStatus } from '@/rpc/api/quotes/v1/models_pb'

/** Drafts, and sent quotes nobody signed yet: a signature binds the quote as it was signed. */
export const isQuoteEditable = (quote: Pick<DetailedQuote, 'quote' | 'signatures'>) =>
  quote.quote?.status === QuoteStatus.DRAFT ||
  (quote.quote?.status === QuoteStatus.PENDING && quote.signatures.length === 0)
