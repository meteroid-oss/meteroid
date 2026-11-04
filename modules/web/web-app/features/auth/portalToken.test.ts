import { describe, expect, it } from 'vitest'

import { portalCheckoutHref } from '@/features/checkout/utils/checkoutRedirect'
import { withRestoredToken } from '@/features/checkout/utils/hostedReturn'
import {
  topUpReturnUrl,
  withStoredPortalToken,
} from '@/pages/portal/experience/modals/topUpCheckout'

import {
  clearAllPortalTokens,
  clearPortalToken,
  opensCustomerPortal,
  pagePortalToken,
  portalTokenResource,
  portalTokenScope,
  readPortalToken,
} from './portalToken'

const memoryStorage = () => {
  const items = new Map<string, string>()
  return {
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => void items.set(key, value),
    removeItem: (key: string) => void items.delete(key),
    keys: () => Array.from(items.keys()),
  }
}

const at = (href: string) => {
  const url = new URL(href)
  return { pathname: url.pathname, search: url.search }
}

const jwt = (resource: Record<string, unknown>, scopes?: string[]) =>
  [
    'eyJhbGciOiJIUzI1NiJ9',
    btoa(JSON.stringify({ tenant_id: 't', resource, scopes }))
      .replace(/=+$/, '')
      .replace(/\+/g, '-')
      .replace(/\//g, '_'),
    'sig',
  ].join('.')

const PORTAL = 'https://billing.example.com/portal/customer?token=tok_portal&page=balance'

describe('portalTokenScope', () => {
  it('gives every portal page kind its own token', () => {
    expect(portalTokenScope('/portal/customer')).toBe('customer')
    expect(portalTokenScope('/portal/subscription/sub_1')).toBe('customer')
    expect(portalTokenScope('/checkout')).toBe('checkout')
    expect(portalTokenScope('/checkout/sub_1')).toBe('checkout')
    expect(portalTokenScope('/portal/checkout')).toBe('checkout')
    expect(portalTokenScope('/portal/invoice-payment/inv_1')).toBe('invoice')
    expect(portalTokenScope('/portal/quote')).toBe('quote')
    expect(portalTokenScope('/portal/dashboard')).toBe('dashboard')
    expect(portalTokenScope('/checkoutx')).toBeUndefined()
  })

  it('has no scope off the portal, where ?token= means something else', () => {
    for (const path of ['/reset-password', '/mfa-challenge', '/validate-email', '/oauth_success', '/', '/portal']) {
      expect(portalTokenScope(path)).toBeUndefined()
    }
    const storage = memoryStorage()
    expect(pagePortalToken(at('https://billing.example.com/reset-password?token=reset'), storage)).toBeNull()
    expect(storage.keys()).toEqual([])
  })
})

describe('pagePortalToken', () => {
  it('keeps the URL token for its page and uses it when the URL has none', () => {
    const storage = memoryStorage()
    expect(pagePortalToken(at(PORTAL), storage)).toBe('tok_portal')
    expect(pagePortalToken(at('https://billing.example.com/portal/customer'), storage)).toBe(
      'tok_portal'
    )
  })

  it('never lets a checkout token replace the portal token (top-up with a new card)', () => {
    const storage = memoryStorage()
    pagePortalToken(at(PORTAL), storage)
    pagePortalToken(at('https://billing.example.com/checkout?token=tok_checkout'), storage)

    const back = topUpReturnUrl(PORTAL, 'wal_1')
    expect(pagePortalToken(at(back), storage)).toBe('tok_portal')
    expect(
      withStoredPortalToken(back, readPortalToken(portalTokenScope('/portal/customer'), storage))
    ).toBe(`${back}&token=tok_portal`)
    expect(readPortalToken('checkout', storage)).toBe('tok_checkout')
  })

  it('returns a plan change or add-on checkout to the portal with the portal token', () => {
    const storage = memoryStorage()
    pagePortalToken(at(PORTAL), storage)
    const checkout = new URL(
      portalCheckoutHref('tok_checkout', PORTAL),
      'https://billing.example.com'
    )
    pagePortalToken(at(checkout.href), storage)

    const back = checkout.searchParams.get('return_url')!
    expect(back).not.toContain('tok_portal')
    expect(
      withRestoredToken(back, readPortalToken(portalTokenScope(new URL(back).pathname), storage))
    ).toBe('https://billing.example.com/portal/customer?page=balance&token=tok_portal')
  })

  it('keeps an invoice payment token apart from the portal token', () => {
    const storage = memoryStorage()
    pagePortalToken(at(PORTAL), storage)
    pagePortalToken(
      at('https://billing.example.com/portal/invoice-payment/inv_1?token=tok_inv'),
      storage
    )

    expect(pagePortalToken(at('https://billing.example.com/portal/customer'), storage)).toBe(
      'tok_portal'
    )
    expect(
      pagePortalToken(at('https://billing.example.com/portal/invoice-payment/inv_1'), storage)
    ).toBe('tok_inv')
  })

  it('is null without a URL or kept token, and when storage is unavailable', () => {
    expect(
      pagePortalToken(at('https://billing.example.com/portal/customer'), memoryStorage())
    ).toBeNull()
    expect(pagePortalToken(at(PORTAL), undefined)).toBe('tok_portal')
    const throwing = {
      getItem: () => {
        throw new Error('blocked')
      },
      setItem: () => {
        throw new Error('blocked')
      },
      removeItem: () => {},
    }
    expect(pagePortalToken(at(PORTAL), throwing)).toBe('tok_portal')
    expect(pagePortalToken(at('https://billing.example.com/portal/customer'), throwing)).toBeNull()
  })
})

describe('clearPortalToken', () => {
  it('clears one page kind on a rejected token, and every kind on logout', () => {
    const storage = memoryStorage()
    storage.setItem('portal-token', 'legacy')
    pagePortalToken(at(PORTAL), storage)
    pagePortalToken(at('https://billing.example.com/checkout?token=tok_checkout'), storage)

    clearPortalToken('checkout', storage)
    expect(readPortalToken('checkout', storage)).toBeNull()
    expect(readPortalToken('customer', storage)).toBe('tok_portal')

    clearPortalToken(undefined, storage)
    expect(readPortalToken('customer', storage)).toBe('tok_portal')

    clearAllPortalTokens(storage)
    expect(storage.keys()).toEqual([])
  })
})

describe('portalTokenResource', () => {
  it('reads the resource kind of the token', () => {
    expect(portalTokenResource(jwt({ Customer: 'cus_1' }))).toBe('customer')
    expect(portalTokenResource(jwt({ Invoice: 'inv_1' }))).toBe('invoice')
    expect(portalTokenResource(jwt({ CheckoutSession: 'cs_1' }))).toBe('checkout')
    expect(portalTokenResource(jwt({ Quote: { quote_id: 'q', recipient_email: 'a@b.c' } }))).toBe(
      'quote'
    )
  })

  it('is undefined for anything else', () => {
    expect(portalTokenResource('not-a-jwt')).toBeUndefined()
    expect(portalTokenResource('a.%%%.c')).toBeUndefined()
    expect(portalTokenResource(jwt({ Other: 'x' }))).toBeUndefined()
  })
})

describe('opensCustomerPortal', () => {
  it('is a customer token without restricting scopes, or with read or manage', () => {
    expect(opensCustomerPortal(jwt({ Customer: 'cus_1' }))).toBe(true)
    expect(opensCustomerPortal(jwt({ Customer: 'cus_1' }, ['read']))).toBe(true)
    expect(opensCustomerPortal(jwt({ Customer: 'cus_1' }, ['manage']))).toBe(true)
  })

  it('is false for a customer token without portal scopes, or another resource', () => {
    expect(opensCustomerPortal(jwt({ Customer: 'cus_1' }, []))).toBe(false)
    expect(opensCustomerPortal(jwt({ Invoice: 'inv_1' }))).toBe(false)
    expect(opensCustomerPortal('not-a-jwt')).toBe(false)
  })
})
