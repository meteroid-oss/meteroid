import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import { formatShortDay } from '@/features/subscriptions/utils/dates'

import { formatDateOnly, parseAndFormatDate } from './date'

describe('date fields', () => {
  const tz = process.env.TZ
  beforeAll(() => {
    process.env.TZ = 'America/Los_Angeles'
  })
  afterAll(() => {
    if (tz === undefined) delete process.env.TZ
    else process.env.TZ = tz
  })

  it('render a yyyy-MM-dd date as that calendar day west of UTC', () => {
    expect(parseAndFormatDate('2026-10-30')).toBe('Oct 30, 2026')
    expect(formatDateOnly('2026-10-30')).toBe('Oct 30, 2026')
  })

  it('read a naive timestamp as UTC', () => {
    process.env.TZ = 'Asia/Tokyo'
    try {
      expect(formatDateOnly('2026-10-30T00:00:00')).toBe('Oct 30, 2026')
      expect(formatShortDay('2026-10-30T00:00:00')).toMatch(/^Oct 30/)
    } finally {
      process.env.TZ = 'America/Los_Angeles'
    }
  })
})
