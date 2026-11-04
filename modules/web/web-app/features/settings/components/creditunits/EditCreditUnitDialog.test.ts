import { describe, expect, it } from 'vitest'

import { editCreditUnitSchema } from './EditCreditUnitDialog'

const ok = (fields: Partial<Record<'name' | 'pluralName' | 'symbol', string>>) =>
  editCreditUnitSchema.safeParse({ name: 'request', pluralName: 'requests', symbol: '', ...fields })
    .success

describe('credit unit edit', () => {
  it('bounds names and symbol in bytes once trimmed, like the server', () => {
    expect(ok({})).toBe(true)
    expect(ok({ name: '   ' })).toBe(false)
    expect(ok({ pluralName: 'x'.repeat(64) })).toBe(true)
    expect(ok({ pluralName: 'é'.repeat(33) })).toBe(false)
    expect(ok({ symbol: ' 12345678 ' })).toBe(true)
    expect(ok({ symbol: '€€€' })).toBe(false)
  })
})
