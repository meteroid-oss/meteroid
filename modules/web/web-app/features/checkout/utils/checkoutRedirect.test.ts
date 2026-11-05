import { describe, expect, it } from 'vitest'

import { legacyReturnUrl } from './checkoutRedirect'

describe('legacyReturnUrl', () => {
  it('ignores a javascript: return_url', () => {
    expect(
      legacyReturnUrl(new URLSearchParams({ return_url: 'javascript:alert(1)' }))
    ).toBeUndefined()
  })

  it('passes through an https return_url', () => {
    expect(
      legacyReturnUrl(new URLSearchParams({ return_url: 'https://merchant.example.com/x' }))
    ).toBe('https://merchant.example.com/x')
  })
})
