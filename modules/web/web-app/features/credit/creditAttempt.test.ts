import { Code, ConnectError } from '@connectrpc/connect'
import { describe, expect, it } from 'vitest'

import {
  addMonthsUtc,
  attemptPayload,
  nextCreditAttempt,
  settleCreditAttempt,
} from './creditAttempt'

const keys = () => {
  let n = 0
  return () => `key-${++n}`
}

describe('nextCreditAttempt', () => {
  const opened = new Date('2026-01-31T10:00:00Z')

  it('retries the same request under the same key and frozen expiry', () => {
    const newKey = keys()
    const first = nextCreditAttempt(undefined, 'p1', opened, newKey)
    if (!('attempt' in first)) throw new Error('expected an attempt')
    const declined = settleCreditAttempt(
      first.attempt,
      new ConnectError('Your card was declined.', Code.FailedPrecondition)
    )
    const retry = nextCreditAttempt(declined, 'p1', new Date('2026-02-01T00:00:00Z'), newKey)
    expect(retry).toEqual({ attempt: { ...first.attempt, outcome: 'sent' } })
  })

  it('gives a changed request a new key and expiry once the last one was refused', () => {
    const newKey = keys()
    const first = nextCreditAttempt(undefined, 'p1', opened, newKey)
    if (!('attempt' in first)) throw new Error('expected an attempt')
    const refused = settleCreditAttempt(
      first.attempt,
      new ConnectError('declined', Code.FailedPrecondition)
    )
    const later = new Date('2026-02-01T00:00:00Z')
    expect(nextCreditAttempt(refused, 'p2', later, newKey)).toEqual({
      attempt: { key: 'key-2', frozenAt: later, payload: 'p2', outcome: 'sent' },
    })
  })

  it('refuses a changed request after an unknown outcome: it could charge twice', () => {
    const newKey = keys()
    const first = nextCreditAttempt(undefined, 'p1', opened, newKey)
    if (!('attempt' in first)) throw new Error('expected an attempt')
    for (const code of [Code.Unavailable, Code.Internal, Code.DeadlineExceeded, Code.Unknown]) {
      const unknown = settleCreditAttempt(first.attempt, new ConnectError('lost', code))
      expect(nextCreditAttempt(unknown, 'p2', opened, newKey)).toEqual({ blocked: true })
      expect(nextCreditAttempt(unknown, 'p1', opened, newKey)).toEqual({
        attempt: { ...first.attempt, outcome: 'sent' },
      })
    }
  })
})

describe('addMonthsUtc', () => {
  it('keeps the day and time, clamped to the last day of the target month', () => {
    expect(addMonthsUtc(new Date('2026-01-31T10:30:00Z'), 1).toISOString()).toBe(
      '2026-02-28T10:30:00.000Z'
    )
    expect(addMonthsUtc(new Date('2028-01-31T00:00:00Z'), 1).toISOString()).toBe(
      '2028-02-29T00:00:00.000Z'
    )
    expect(addMonthsUtc(new Date('2026-10-17T23:59:59.500Z'), 3).toISOString()).toBe(
      '2027-01-17T23:59:59.500Z'
    )
    expect(addMonthsUtc(new Date('2026-03-31T12:00:00Z'), 120).toISOString()).toBe(
      '2036-03-31T12:00:00.000Z'
    )
  })
})

describe('attemptPayload', () => {
  it('is stable across key order and serializes amounts', () => {
    expect(attemptPayload({ amount: 100n, months: 3 })).toBe(
      attemptPayload({ months: 3, amount: 100n })
    )
    expect(attemptPayload({ amount: 100n })).not.toBe(attemptPayload({ amount: 101n }))
  })
})
