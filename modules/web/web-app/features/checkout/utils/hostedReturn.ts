import { portalTokenScope, readPortalToken } from '@/features/auth/portalToken'

/**
 * Return-URL contract for all hosted-redirect providers: the return handler redirects back with
 * `hosted_status` and, on failure, `hosted_error`. `ok` = method saved (and any payment done or,
 * for webhook-driven providers, submitted); `processing` = still settling; `payment_failed` =
 * method saved but the first charge declined; `failed` / `abandoned` = nothing saved.
 */

export type HostedOutcome = 'ok' | 'processing' | 'payment_failed' | 'failed' | 'abandoned'

export type HostedRail = 'card' | 'directDebit'

/** What the page knew when the customer left for the hosted flow. */
export interface HostedDeparture {
  rail: HostedRail
  /** Payment methods on file before departure; anything else is new on return. */
  paymentMethodIds?: string[]
}

export interface HostedReturn {
  status: HostedOutcome
  error?: string
  // Absent when nothing was saved (resumed attempt, storage blocked).
  departure?: HostedDeparture
}

const isHostedOutcome = (v: string | null): v is HostedOutcome =>
  v === 'ok' || v === 'processing' || v === 'payment_failed' || v === 'failed' || v === 'abandoned'

export const HOSTED_STATUS_PARAM = 'hosted_status'
const RETURN_PARAMS = [HOSTED_STATUS_PARAM, 'hosted_error']

/**
 * Read the hosted-flow return outcome from the current URL and strip the
 * provider params (via replaceState) so a reload doesn't replay it. The
 * portal `?token=` and every other param are preserved.
 */
// The first read strips the URL; a re-run initializer (StrictMode) gets the same result.
let lastConsumed: { url: string; ret: HostedReturn; at: number } | null = null
const CONSUMED_REUSE_MS = 10_000

export const consumeHostedReturn = (): HostedReturn | null => {
  if (typeof window === 'undefined') return null

  const currentUrl = `${window.location.pathname}${window.location.search}${window.location.hash}`
  if (lastConsumed && lastConsumed.url === currentUrl && Date.now() - lastConsumed.at < CONSUMED_REUSE_MS) {
    return lastConsumed.ret
  }

  const params = new URLSearchParams(window.location.search)
  const status = params.get(HOSTED_STATUS_PARAM)
  if (!isHostedOutcome(status)) return null
  const ret: HostedReturn = { status, error: params.get('hosted_error') ?? undefined }

  RETURN_PARAMS.forEach(p => params.delete(p))
  const search = params.toString()
  const nextUrl = `${window.location.pathname}${search ? `?${search}` : ''}${window.location.hash}`
  window.history.replaceState(window.history.state, '', nextUrl)

  const departure = consumeHostedDeparture()
  const consumed = departure ? { ...ret, departure } : ret
  lastConsumed = { url: nextUrl, ret: consumed, at: Date.now() }
  return consumed
}

const DEPARTURE_KEY = 'hosted_departure'
// An older departure is ignored so a stale snapshot can't hide a real failure on a later visit.
const HOSTED_STASH_TTL_MS = 60 * 60 * 1000

/** Save the rail (and known methods) before redirecting; the return carries neither. */
export const stashHostedDeparture = (departure: HostedDeparture): void => {
  if (typeof window === 'undefined') return
  try {
    window.sessionStorage.setItem(DEPARTURE_KEY, JSON.stringify({ ...departure, ts: Date.now() }))
  } catch {
    // sessionStorage can throw (private mode, quota); messages then stay rail-neutral.
  }
}

const consumeHostedDeparture = (): HostedDeparture | undefined => {
  try {
    const raw = window.sessionStorage.getItem(DEPARTURE_KEY)
    if (!raw) return undefined
    window.sessionStorage.removeItem(DEPARTURE_KEY)
    const parsed = JSON.parse(raw) as { rail?: unknown; paymentMethodIds?: unknown; ts?: unknown }
    if (typeof parsed.ts !== 'number' || Date.now() - parsed.ts > HOSTED_STASH_TTL_MS) return undefined
    if (parsed.rail !== 'card' && parsed.rail !== 'directDebit') return undefined
    return {
      rail: parsed.rail,
      paymentMethodIds: Array.isArray(parsed.paymentMethodIds)
        ? parsed.paymentMethodIds.filter((id): id is string => typeof id === 'string')
        : undefined,
    }
  } catch {
    return undefined
  }
}

const CREDENTIAL_PARAM =
  /token|secret|passw|session|auth|signature|credential|api[-_]?key|^key$|^sig$|^code$/i
/** Set on a return URL whose credentials were stripped: the page puts its stored token back. */
export const HOSTED_RETURN_MARKER = 'hosted_return'

/**
 * Where a provider sends the customer back: this page, without stale provider params and without
 * the page token or any other credential, which the provider would store. The tab keeps the token
 * in its session (the auth interceptor saves it) and restores it on return.
 */
export const hostedReturnTarget = (href: string): string => {
  const url = new URL(href)
  url.username = ''
  url.password = ''
  url.hash = ''
  RETURN_PARAMS.forEach(p => url.searchParams.delete(p))
  url.searchParams.delete(HOSTED_RETURN_MARKER)
  let stripped = false
  for (const name of Array.from(new Set(url.searchParams.keys()))) {
    if (CREDENTIAL_PARAM.test(name)) {
      url.searchParams.delete(name)
      stripped = true
    }
  }
  if (stripped) url.searchParams.set(HOSTED_RETURN_MARKER, '1')
  return url.toString()
}

export const hostedReturnUrl = (): string | undefined =>
  typeof window === 'undefined' ? undefined : hostedReturnTarget(window.location.href)

/** The returned page's URL with the token the tab kept; null when there is nothing to change. */
export const withRestoredToken = (href: string, storedToken: string | null): string | null => {
  const url = new URL(href)
  if (!url.searchParams.has(HOSTED_RETURN_MARKER)) return null
  url.searchParams.delete(HOSTED_RETURN_MARKER)
  // Without a stored token the page shows its expired-session state.
  if (storedToken && !url.searchParams.has('token')) url.searchParams.set('token', storedToken)
  return url.toString()
}

const restoreTokenAfterHostedReturn = () => {
  if (typeof window === 'undefined') return
  const stored = readPortalToken(portalTokenScope(window.location.pathname))
  const restored = withRestoredToken(window.location.href, stored)
  if (restored) window.history.replaceState(window.history.state, '', restored)
}

// On import, so before the router and the pages first read the location.
restoreTokenAfterHostedReturn()

const PRE_ATTEMPT_KEY = (invoiceId: string) => `hosted_pre_attempt_failed:${invoiceId}`

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
    if (typeof parsed.ts !== 'number' || Date.now() - parsed.ts > HOSTED_STASH_TTL_MS) return null
    if (!Array.isArray(parsed.ids)) return null
    return new Set(parsed.ids.filter((id): id is string => typeof id === 'string'))
  } catch {
    return null
  }
}

const setupNoun = (rail?: HostedRail) =>
  rail === 'directDebit' ? 'Direct debit mandate' : rail === 'card' ? 'Card' : 'Payment method'

const setupLabel = (rail?: HostedRail) =>
  rail === 'directDebit' ? 'Direct debit setup' : rail === 'card' ? 'Card setup' : 'Payment method setup'

/** User-facing message for a hosted payment-method setup that saved the method. */
export const hostedReturnSuccessMessage = (ret: HostedReturn): string =>
  `${setupNoun(ret.departure?.rail)} saved.`

/** User-facing message for a non-`ok` hosted return, worded for the rail the customer left
 *  from (saved before departure; the return itself is provider-neutral). */
export const hostedReturnErrorMessage = (ret: HostedReturn): string => {
  const rail = ret.departure?.rail
  switch (ret.status) {
    case 'processing':
      return `Your ${setupNoun(rail).toLowerCase()} is still being confirmed. Please wait a moment and try again.`
    case 'payment_failed':
      return `Your ${setupNoun(rail).toLowerCase()} was saved, but the payment was declined. Please retry with it or use a different payment method.`
    case 'abandoned':
      return `${setupLabel(rail)} was cancelled. You can try again.`
    default:
      return ret.error
        ? `${setupLabel(rail)} failed (${ret.error}). Please try again.`
        : `${setupLabel(rail)} failed. Please try again.`
  }
}
