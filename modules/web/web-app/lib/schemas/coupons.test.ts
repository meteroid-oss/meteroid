import { describe, expect, it } from 'vitest'

import { createCouponSchema } from '@/lib/schemas/coupons'

const percentage = (code: string) => ({ code, discountType: 'percentage', percentage: 10 })
const credit = (code: string) => ({
  code,
  discountType: 'credit',
  creditAmount: '10',
  currency: 'EUR',
})
const codeIssues = (input: unknown) => {
  const result = createCouponSchema.safeParse(input)
  return result.success ? [] : result.error.issues.filter(issue => issue.path[0] === 'code')
}

describe('coupon code', () => {
  it('accepts the 1 to 64 characters the server accepts', () => {
    expect(codeIssues(percentage('A'))).toEqual([])
    expect(codeIssues(percentage('x'.repeat(64)))).toEqual([])
    expect(codeIssues(percentage('WELCOME 2026'))).toEqual([])
  })

  it('refuses an empty or longer code', () => {
    expect(codeIssues(percentage(''))).toHaveLength(1)
    expect(codeIssues(percentage('   '))).toHaveLength(1)
    expect(codeIssues(percentage('x'.repeat(65)))).toHaveLength(1)
  })

  it('sends the code without surrounding whitespace', () => {
    const result = createCouponSchema.safeParse(percentage('  LAUNCH20 '))
    expect(result.success && result.data.code).toBe('LAUNCH20')
  })

  it('limits a promotional credit code to letters, digits, hyphens and underscores', () => {
    expect(codeIssues(credit('welcome_10-B'))).toEqual([])
    expect(codeIssues(credit('WELCOME 2026'))).toHaveLength(1)
    expect(codeIssues(credit('CAFÉ'))).toHaveLength(1)
  })
})
