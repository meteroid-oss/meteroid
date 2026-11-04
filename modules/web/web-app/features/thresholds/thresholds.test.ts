import { describe, expect, it } from 'vitest'

import { MAX_THRESHOLDS, formatThresholds, parseThresholds, thresholdsSchema } from './thresholds'

describe('alert thresholds', () => {
  it('read increasing whole percentages, empty for none', () => {
    expect(parseThresholds('')).toEqual([])
    expect(parseThresholds(' 50, 80 ,95 ')).toEqual([50, 80, 95])
    expect(formatThresholds([50, 80])).toBe('50, 80')
  })

  it('refuse values out of 1-100, unordered, repeated or too many', () => {
    for (const text of ['0', '101', '80, 50', '50, 50', '5.5', 'x']) {
      expect(thresholdsSchema.safeParse(text).success).toBe(false)
    }
    const many = Array.from({ length: MAX_THRESHOLDS + 1 }, (_, i) => i + 1).join(',')
    expect(thresholdsSchema.safeParse(many).success).toBe(false)
  })
})
