export type GocardlessOutcome = 'ok' | 'failed' | 'abandoned'

export interface GocardlessReturn {
  status: GocardlessOutcome
  /** Sanitized provider error code, present on some `failed` returns. */
  error?: string
}

const isOutcome = (v: string | null): v is GocardlessOutcome =>
  v === 'ok' || v === 'failed' || v === 'abandoned'

/**
 * Read the GoCardless return outcome from the current URL and strip the
 * `gocardless_*` params (via replaceState) so a reload or Back navigation
 * doesn't replay it. Returns null when there's no outcome to handle.
 *
 * The portal `?token=` and every other param are preserved.
 */
export const consumeGocardlessReturn = (): GocardlessReturn | null => {
  if (typeof window === 'undefined') return null

  const params = new URLSearchParams(window.location.search)
  const status = params.get('gocardless_status')
  if (!isOutcome(status)) return null

  const error = params.get('gocardless_error') ?? undefined

  params.delete('gocardless_status')
  params.delete('gocardless_error')
  const search = params.toString()
  const nextUrl = `${window.location.pathname}${search ? `?${search}` : ''}${window.location.hash}`
  window.history.replaceState(window.history.state, '', nextUrl)

  return { status, error }
}

/**
 * The current page URL to hand a GoCardless setup intent as its return target,
 * with any stale `gocardless_*` params removed so they don't accumulate across
 * retries. Keeps the portal `?token=` so the customer returns authenticated.
 */
export const gocardlessReturnUrl = (): string | undefined => {
  if (typeof window === 'undefined') return undefined
  const url = new URL(window.location.href)
  url.searchParams.delete('gocardless_status')
  url.searchParams.delete('gocardless_error')
  return url.toString()
}

const PRE_ATTEMPT_KEY = (invoiceId: string) => `gc_pre_attempt_failed:${invoiceId}`
// A departure older than this can't belong to the mandate the user just
// authorised, so we ignore it rather than let a stale snapshot suppress a real
// failure on a much later, unrelated visit.
const PRE_ATTEMPT_TTL_MS = 60 * 60 * 1000

/**
 * Record which transactions were already FAILED *before* the customer leaves for
 * the GoCardless hosted flow, so the return handler can tell a genuinely new
 * charge failure apart from those pre-existing attempts.
 *
 * Seeding the stale set from the first poll after return is racy: if the webhook
 * creates and fails the new charge before that poll resolves, the fresh failure
 * would be mistaken for a pre-existing one. This snapshot is captured before the
 * charge can exist, so it's race-free. Keyed by invoice; the latest departure
 * wins.
 */
export const stashGocardlessPreAttempt = (invoiceId: string, failedTxIds: string[]): void => {
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
export const consumeGocardlessPreAttempt = (invoiceId: string): Set<string> | null => {
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

/** User-facing message for a non-success GoCardless return. */
export const gocardlessErrorMessage = (ret: GocardlessReturn): string => {
  if (ret.status === 'abandoned') {
    return 'Direct debit authorisation was cancelled. You can try again.'
  }
  return ret.error
    ? `Direct debit authorisation failed (${ret.error}). Please try again.`
    : 'Direct debit authorisation failed. Please try again.'
}
