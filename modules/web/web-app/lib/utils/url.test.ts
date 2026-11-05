import { describe, expect, it } from 'vitest'

import { safeRedirectUrl } from './url'

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
