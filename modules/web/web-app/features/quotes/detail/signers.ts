import { ConnectError } from '@connectrpc/connect'

import { DetailedQuote, QuoteSignerRole } from '@/rpc/api/quotes/v1/models_pb'

/** Mirrors the server's default, shown when the quote sets no consent text. */
export const DEFAULT_CONSENT_TEXT =
  'I have read this quote and accept it, with its terms and pricing, on behalf of my organization.'

export const DEFAULT_CONSENT_TEXT_WITH_ATTACHMENTS =
  'I have read this quote and its attached documents and accept them, with their terms and pricing, on behalf of my organization.'

export const defaultConsentText = (hasAttachments: boolean) =>
  hasAttachments ? DEFAULT_CONSENT_TEXT_WITH_ATTACHMENTS : DEFAULT_CONSENT_TEXT

export const QUOTE_CHANGED_MESSAGE = 'The quote changed since it was opened'

export const rpcErrorMessage = (error: unknown, fallback: string) =>
  ConnectError.from(error).rawMessage || fallback

export const signerRoleLabel = (role: QuoteSignerRole) =>
  role === QuoteSignerRole.SIGNER_COUNTERSIGNER ? 'Countersigner' : 'Customer'

export const sameEmail = (a?: string, b?: string) =>
  Boolean(a && b && a.trim().toLowerCase() === b.trim().toLowerCase())

export const hasSignedAs = (
  signatures: { signedByEmail: string; signerRole: QuoteSignerRole }[],
  email: string,
  role: QuoteSignerRole
) => signatures.some(s => s.signerRole === role && sameEmail(s.signedByEmail, email))

/** The countersigner the user can sign as: the customer has signed and they have not yet. */
export const pendingCountersignerFor = (quote: DetailedQuote, userEmail?: string) => {
  const q = quote.quote
  if (!q || !userEmail) return undefined
  const countersigner = q.countersigners.find(c => !c.automatic && sameEmail(c.email, userEmail))
  if (!countersigner) return undefined
  const customersSigned = q.recipients.every(r =>
    hasSignedAs(quote.signatures, r.email, QuoteSignerRole.SIGNER_CUSTOMER)
  )
  const alreadySigned = hasSignedAs(
    quote.signatures,
    countersigner.email,
    QuoteSignerRole.SIGNER_COUNTERSIGNER
  )
  return customersSigned && !alreadySigned ? countersigner : undefined
}
