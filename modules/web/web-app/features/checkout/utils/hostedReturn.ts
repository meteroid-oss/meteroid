/**
 * Return-URL contract shared by the hosted-redirect payment providers.
 *
 * GoCardless (direct debit) and Stancer (card) both send the customer to a
 * provider-hosted page and bounce them back through a server-side return
 * handler, which redirects to the original page with a `<provider>_status`
 * (and sometimes `<provider>_error`) query param. The outcome vocabularies
 * differ because the money paths differ:
 *
 * - GoCardless: `ok | failed | abandoned`. Completion is webhook-driven; `ok`
 *   only means the mandate was authorised.
 * - Stancer: `ok | processing | payment_failed | failed`. There is no webhook —
 *   the return handler IS the completion path. `ok` = card saved (and any
 *   first charge initiated); `payment_failed` = card saved but the first
 *   charge was declined (retry with the saved card); `processing` = the card
 *   isn't confirmed yet (the flow is idempotent — retrying is safe).
 */

export type HostedReturnProvider = 'gocardless' | 'stancer'

export type GocardlessOutcome = 'ok' | 'failed' | 'abandoned'
export type StancerOutcome = 'ok' | 'processing' | 'payment_failed' | 'failed'

export type HostedReturn =
  | { provider: 'gocardless'; status: GocardlessOutcome; error?: string }
  | { provider: 'stancer'; status: StancerOutcome; error?: string }

const isGocardlessOutcome = (v: string | null): v is GocardlessOutcome =>
  v === 'ok' || v === 'failed' || v === 'abandoned'

const isStancerOutcome = (v: string | null): v is StancerOutcome =>
  v === 'ok' || v === 'processing' || v === 'payment_failed' || v === 'failed'

const RETURN_PARAMS = ['gocardless_status', 'gocardless_error', 'stancer_status', 'stancer_error']

/**
 * Read the hosted-flow return outcome from the current URL and strip every
 * `gocardless_*` / `stancer_*` param (via replaceState) so a reload or Back
 * navigation doesn't replay it. Returns null when there's no outcome to handle.
 *
 * The portal `?token=` and every other param are preserved.
 */
export const consumeHostedReturn = (): HostedReturn | null => {
  if (typeof window === 'undefined') return null

  const params = new URLSearchParams(window.location.search)
  const gcStatus = params.get('gocardless_status')
  const stancerStatus = params.get('stancer_status')

  let ret: HostedReturn | null = null
  if (isGocardlessOutcome(gcStatus)) {
    ret = { provider: 'gocardless', status: gcStatus, error: params.get('gocardless_error') ?? undefined }
  } else if (isStancerOutcome(stancerStatus)) {
    ret = { provider: 'stancer', status: stancerStatus, error: params.get('stancer_error') ?? undefined }
  }
  if (!ret) return null

  RETURN_PARAMS.forEach(p => params.delete(p))
  const search = params.toString()
  const nextUrl = `${window.location.pathname}${search ? `?${search}` : ''}${window.location.hash}`
  window.history.replaceState(window.history.state, '', nextUrl)

  return ret
}

/**
 * The current page URL to hand a hosted-redirect setup/checkout as its return
 * target, with any stale `gocardless_*` / `stancer_*` params removed so they
 * don't accumulate across retries. Keeps the portal `?token=` so the customer
 * returns authenticated.
 */
export const hostedReturnUrl = (): string | undefined => {
  if (typeof window === 'undefined') return undefined
  const url = new URL(window.location.href)
  RETURN_PARAMS.forEach(p => url.searchParams.delete(p))
  return url.toString()
}

const PRE_ATTEMPT_KEY = (invoiceId: string) => `hosted_pre_attempt_failed:${invoiceId}`
// A departure older than this can't belong to the flow the user just
// completed, so we ignore it rather than let a stale snapshot suppress a real
// failure on a much later, unrelated visit.
const PRE_ATTEMPT_TTL_MS = 60 * 60 * 1000

/**
 * Record which transactions were already FAILED *before* the customer leaves
 * for a provider-hosted flow, so the return handler can tell a genuinely new
 * charge failure apart from those pre-existing attempts.
 *
 * Seeding the stale set from the first poll after return is racy: if the
 * backend creates and fails the new charge before that poll resolves, the
 * fresh failure would be mistaken for a pre-existing one. This snapshot is
 * captured before the charge can exist, so it's race-free. Keyed by invoice;
 * the latest departure wins.
 */
export const stashHostedPreAttempt = (invoiceId: string, failedTxIds: string[]): void => {
  if (typeof window === 'undefined') return
  try {
    window.sessionStorage.setItem(
      PRE_ATTEMPT_KEY(invoiceId),
      JSON.stringify({ ids: failedTxIds, ts: Date.now() })
    )
  } catch {
    // sessionStorage can throw (private mode / quota); fall back to first-poll seeding.
  }
}

/**
 * Read and clear the pre-departure failed-transaction snapshot for an invoice.
 * Returns null when there's no (fresh) snapshot, so the caller falls back to
 * seeding from the first polled invoice.
 */
export const consumeHostedPreAttempt = (invoiceId: string): Set<string> | null => {
  if (typeof window === 'undefined') return null
  try {
    const raw = window.sessionStorage.getItem(PRE_ATTEMPT_KEY(invoiceId))
    if (!raw) return null
    window.sessionStorage.removeItem(PRE_ATTEMPT_KEY(invoiceId))
    const parsed = JSON.parse(raw) as { ids?: unknown; ts?: unknown }
    if (typeof parsed.ts !== 'number' || Date.now() - parsed.ts > PRE_ATTEMPT_TTL_MS) return null
    if (!Array.isArray(parsed.ids)) return null
    return new Set(parsed.ids.filter((id): id is string => typeof id === 'string'))
  } catch {
    return null
  }
}

/** User-facing message for a non-`ok` hosted-flow return. */
export const hostedReturnErrorMessage = (ret: HostedReturn): string => {
  if (ret.provider === 'gocardless') {
    if (ret.status === 'abandoned') {
      return 'Direct debit authorisation was cancelled. You can try again.'
    }
    return ret.error
      ? `Direct debit authorisation failed (${ret.error}). Please try again.`
      : 'Direct debit authorisation failed. Please try again.'
  }
  // Stancer
  switch (ret.status) {
    case 'processing':
      return 'Your card details are still being confirmed. Please wait a moment and try again.'
    case 'payment_failed':
      return 'Your card was saved, but the payment was declined. Please retry with this card or use a different one.'
    default:
      return ret.error
        ? `Card setup failed (${ret.error}). Please try again.`
        : 'Card setup failed. Please try again.'
  }
}
