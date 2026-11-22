import { describe, expect, it } from 'vitest'

import { safeRedirectUrl, sameOriginPath } from './url'

describe('safeRedirectUrl', () => {
  it('keeps http(s) and relative targets', () => {
    expect(safeRedirectUrl('https://merchant.example.com/done?x=1')).toBe(
      'https://merchant.example.com/done?x=1'
    )
    expect(safeRedirectUrl('http://localhost:3000/back')).toBe('http://localhost:3000/back')
    expect(safeRedirectUrl('/portal/customer?page=invoices')).toBe(
      'http://localhost/portal/customer?page=invoices'
    )
  })

  it('drops script and data schemes, however they are cased or padded', () => {
    expect(safeRedirectUrl('javascript:alert(document.domain)')).toBeUndefined()
    expect(safeRedirectUrl(' JaVaScRiPt:alert(1)')).toBeUndefined()
    expect(safeRedirectUrl('java\tscript:alert(1)')).toBeUndefined()
    expect(safeRedirectUrl('data:text/html,<script>alert(1)</script>')).toBeUndefined()
    expect(safeRedirectUrl('vbscript:msgbox(1)')).toBeUndefined()
  })

  it('treats empty input as no target', () => {
    expect(safeRedirectUrl(undefined)).toBeUndefined()
    expect(safeRedirectUrl(null)).toBeUndefined()
    expect(safeRedirectUrl('')).toBeUndefined()
  })
})

describe('sameOriginPath', () => {
  it('keeps in-app paths with their query and hash', () => {
    expect(sameOriginPath('/tenant/acme/customers?page=2#top')).toBe(
      '/tenant/acme/customers?page=2#top'
    )
    expect(sameOriginPath('/')).toBe('/')
  })

  it('refuses targets that resolve to another origin', () => {
    expect(sameOriginPath('//evil.example')).toBeUndefined()
    expect(sameOriginPath('/\\evil.example')).toBeUndefined()
    expect(sameOriginPath('/.\\/evil.example')).toBeUndefined()
    expect(sameOriginPath('/.//evil.example/x')).toBeUndefined()
    expect(sameOriginPath('/%2e/\\evil.example')).toBeUndefined()
    expect(sameOriginPath('https://evil.example/')).toBeUndefined()
    expect(sameOriginPath('javascript:alert(1)')).toBeUndefined()
  })

  it('treats empty input as no target', () => {
    expect(sameOriginPath(undefined)).toBeUndefined()
    expect(sameOriginPath(null)).toBeUndefined()
    expect(sameOriginPath('')).toBeUndefined()
  })
})
