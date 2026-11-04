import { Code, ConnectError } from '@connectrpc/connect'

/**
 * One operation of a dialog under one idempotency key. The expiry is resolved from `frozenAt`
 * once, so every retry under the key sends the very same request.
 */
export type CreditAttempt = {
  key: string
  frozenAt: Date
  payload: string
  /** `refused`: the server answered and did nothing; `unknown`: it may have gone through. */
  outcome: 'sent' | 'refused' | 'unknown'
}

export type NextAttempt = { attempt: CreditAttempt } | { blocked: true }

/**
 * The same request retries under its key. A changed request takes a new key only once the last
 * one was refused: after an unknown outcome it could add credit or charge a second time.
 */
export const nextCreditAttempt = (
  previous: CreditAttempt | undefined,
  payload: string,
  now: Date,
  newKey: () => string
): NextAttempt => {
  if (previous?.payload === payload) return { attempt: { ...previous, outcome: 'sent' } }
  if (!previous || previous.outcome === 'refused') {
    return { attempt: { key: newKey(), frozenAt: now, payload, outcome: 'sent' } }
  }
  return { blocked: true }
}

// Answers after which the server recorded nothing for the key's request.
const REFUSALS = new Set<Code>([
  Code.InvalidArgument,
  Code.FailedPrecondition,
  Code.OutOfRange,
  Code.NotFound,
  Code.AlreadyExists,
  Code.PermissionDenied,
  Code.Unauthenticated,
  Code.ResourceExhausted,
  Code.Aborted,
  Code.Unimplemented,
])

export const settleCreditAttempt = (attempt: CreditAttempt, error: unknown): CreditAttempt => ({
  ...attempt,
  outcome: REFUSALS.has(ConnectError.from(error).code) ? 'refused' : 'unknown',
})

/** `months` after `from` in UTC, the day clamped to the target month's last day, as the server
 * resolves a term in months (chrono's `checked_add_months`). */
export const addMonthsUtc = (from: Date, months: number): Date => {
  const monthIndex = from.getUTCMonth() + months
  const year = from.getUTCFullYear() + Math.floor(monthIndex / 12)
  const month = ((monthIndex % 12) + 12) % 12
  const lastDay = new Date(Date.UTC(year, month + 1, 0)).getUTCDate()
  return new Date(
    Date.UTC(
      year,
      month,
      Math.min(from.getUTCDate(), lastDay),
      from.getUTCHours(),
      from.getUTCMinutes(),
      from.getUTCSeconds(),
      from.getUTCMilliseconds()
    )
  )
}

/** A request's identity for `nextCreditAttempt`: bigint-safe and key-order stable. */
export const attemptPayload = (request: Record<string, unknown>): string =>
  JSON.stringify(
    Object.keys(request)
      .sort()
      .map(key => [key, request[key]]),
    (_, value) => (typeof value === 'bigint' ? value.toString() : value)
  )
