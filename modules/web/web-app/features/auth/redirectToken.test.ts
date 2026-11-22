import { describe, expect, it } from 'vitest'

import { readRedirectToken } from './redirectToken'

describe('readRedirectToken', () => {
  it('reads the fragment first', () => {
    expect(readRedirectToken({ hash: '#token=abc', search: '?token=old' })).toBe('abc')
  })

  it('falls back to the query for links issued before the fragment', () => {
    expect(readRedirectToken({ hash: '', search: '?token=old&returnUrl=%2Fx' })).toBe('old')
  })

  it('is null without a token', () => {
    expect(readRedirectToken({ hash: '', search: '' })).toBeNull()
  })
})
