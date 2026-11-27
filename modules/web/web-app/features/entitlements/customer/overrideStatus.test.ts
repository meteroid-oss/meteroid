import { describe, expect, it } from 'vitest'

import { overrideStatus } from './overrideStatus'

const now = Date.UTC(2026, 11, 1, 12)

describe('where an override stands', () => {
  it('is upcoming before it starts, active without an end', () => {
    expect(overrideStatus({ validFrom: '2026-12-05T00:00:00' }, now)).toBe('upcoming')
    expect(overrideStatus({}, now)).toBe('active')
  })

  it('ends soon within a week of its end, then has ended', () => {
    expect(overrideStatus({ validUntil: '2026-12-08T12:00:00' }, now)).toBe('endsSoon')
    expect(overrideStatus({ validUntil: '2026-12-09T00:00:00' }, now)).toBe('active')
    expect(overrideStatus({ validUntil: '2026-12-01T12:00:00' }, now)).toBe('ended')
  })
})
