/**
 * Return-URL contract of the hosted-redirect providers: the server-side return
 * handler redirects back with `<provider>_status` (and sometimes
 * `<provider>_error`). GoCardless: `ok | failed | abandoned` (webhook-driven;
 * `ok` only means the mandate was authorised). Stancer: `ok | processing |
 * payment_failed | failed` (no webhook — the return handler IS the completion
 * path; `payment_failed` = card saved but the first charge declined).
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
 * Read the hosted-flow return outcome from the current URL and strip the
 * provider params (via replaceState) so a reload doesn't replay it. The
 * portal `?token=` and every other param are preserved.
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
 * The current page URL as a hosted-redirect return target, with stale
 * provider params removed. Keeps the portal `?token=`.
 */
export const hostedReturnUrl = (): string | undefined => {
  if (typeof window === 'undefined') return undefined
  const url = new URL(window.location.href)
  RETURN_PARAMS.forEach(p => url.searchParams.delete(p))
  return url.toString()
}

const PRE_ATTEMPT_KEY = (invoiceId: string) => `hosted_pre_attempt_failed:${invoiceId}`
// An older departure is ignored so a stale snapshot can't suppress a real
// failure on a later, unrelated visit.
const PRE_ATTEMPT_TTL_MS = 60 * 60 * 1000

/**
 * Record which transactions were already FAILED *before* the customer leaves
 * for a hosted flow, so a genuinely new charge failure can be told apart from
 * pre-existing attempts. Seeding from the first poll after return is racy
 * (the backend can create and fail the new charge first); this snapshot is
 * captured before the charge can exist, so it's race-free.
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
 * Read and clear the pre-departure snapshot; null when there's no fresh one
 * (the caller falls back to seeding from the first polled invoice).
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
