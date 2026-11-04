import { describe, expect, it } from 'vitest'

import { monthStart } from './UnattributedUsageSection'

describe('monthStart', () => {
  it('is the first UTC day of the month, across year boundaries', () => {
    const mid = new Date(Date.UTC(2026, 0, 17, 23, 30))
    expect(monthStart(mid)).toBe('2026-01-01')
    expect(monthStart(mid, -1)).toBe('2025-12-01')
    expect(monthStart(mid, 1)).toBe('2026-02-01')
  })
})
