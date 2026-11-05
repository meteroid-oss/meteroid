import { describe, expect, it } from 'vitest'

import { ancestorOrigins, embeddingPermitted } from './embedding'

const portal = 'https://app.meteroid.com'

describe('ancestorOrigins', () => {
  it('reads the frames the browser lists', () => {
    const location = { origin: portal, ancestorOrigins: ['https://acme.com', 'https://top.com'] }
    expect(ancestorOrigins(location, '')).toEqual(['https://acme.com', 'https://top.com'])
  })

  it('falls back on the referrer, ignoring the portal itself', () => {
    expect(ancestorOrigins({ origin: portal }, 'https://acme.com/billing?x=1')).toEqual([
      'https://acme.com',
    ])
    expect(ancestorOrigins({ origin: portal }, `${portal}/portal/customer`)).toEqual([])
    expect(ancestorOrigins({ origin: portal }, '')).toEqual([])
  })
})

describe('embeddingPermitted', () => {
  const registered = ['https://acme.com']

  it('embeds anywhere with the plan', () => {
    expect(embeddingPermitted(true, [], ['https://elsewhere.com'])).toBe(true)
  })

  it('keeps embedding in a registered origin after the plan lapses', () => {
    expect(embeddingPermitted(false, registered, ['https://acme.com'])).toBe(true)
  })

  it('refuses any other or unknown frame without the plan', () => {
    expect(embeddingPermitted(false, registered, ['https://elsewhere.com'])).toBe(false)
    expect(embeddingPermitted(false, registered, ['https://acme.com', 'https://evil.com'])).toBe(
      false
    )
    expect(embeddingPermitted(false, registered, [])).toBe(false)
  })
})
