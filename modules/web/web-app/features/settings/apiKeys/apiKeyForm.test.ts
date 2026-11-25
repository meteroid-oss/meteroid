import { describe, expect, it } from 'vitest'

import { invalidOrigins, isOrigin, parseOrigins } from './apiKeyForm'

describe('allowed origins of a publishable key', () => {
  it('reads one origin per line or comma, each once, as the browser reports it', () => {
    expect(
      parseOrigins(' HTTPS://www.Acme.com/ \nhttps://www.acme.com, http://localhost:3000\n\n')
    ).toEqual(['https://www.acme.com', 'http://localhost:3000'])
    expect(parseOrigins('')).toEqual([])
  })

  it('accepts a scheme, a host and a port only', () => {
    expect(isOrigin('https://www.acme.com')).toBe(true)
    expect(isOrigin('http://localhost:3000')).toBe(true)
    for (const refused of [
      'www.acme.com',
      'https://acme.com/pricing',
      'https://acme.com?x=1',
      'https://acme.com#top',
      'ftp://acme.com',
      'javascript:alert(1)',
      'https://user@acme.com',
    ]) {
      expect(isOrigin(refused), refused).toBe(false)
    }
  })

  it('names the entries the server would refuse', () => {
    expect(invalidOrigins('https://acme.com\nacme.com/pricing')).toEqual(['acme.com/pricing'])
    expect(invalidOrigins('https://acme.com/')).toEqual([])
  })
})
