// Portal bearer tokens, kept per tab and per portal page kind so a provider round trip can
// return without them. A checkout or invoice payment opened from the portal in the same tab
// must never replace the customer's portal token.
export type PortalTokenScope = 'customer' | 'checkout' | 'invoice' | 'quote' | 'dashboard'

const PORTAL_TOKEN_SCOPES: PortalTokenScope[] = [
  'customer',
  'checkout',
  'invoice',
  'quote',
  'dashboard',
]

// Written by builds before the per-page keys; only ever removed.
const LEGACY_STORAGE_KEY = 'portal-token'

const storageKey = (scope: PortalTokenScope) => `portal-token:${scope}`

type TokenStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>

const sessionStore = (): TokenStorage | undefined => {
  try {
    return typeof window === 'undefined' ? undefined : window.sessionStorage
  } catch {
    return undefined
  }
}

/** The portal page kind of a path; none off the portal (reset-password, MFA, … use `?token=` too). */
export const portalTokenScope = (pathname: string): PortalTokenScope | undefined => {
  if (/^\/portal\/(customer|subscription)(\/|$)/.test(pathname)) return 'customer'
  if (/^\/(portal\/)?checkout(\/|$)/.test(pathname)) return 'checkout'
  if (/^\/portal\/invoice-payment(\/|$)/.test(pathname)) return 'invoice'
  if (/^\/portal\/quote(\/|$)/.test(pathname)) return 'quote'
  if (/^\/portal\/dashboard(\/|$)/.test(pathname)) return 'dashboard'
  return undefined
}

export const readPortalToken = (
  scope: PortalTokenScope | undefined,
  storage: TokenStorage | undefined = sessionStore()
): string | null => {
  if (!scope) return null
  try {
    return storage?.getItem(storageKey(scope)) ?? null
  } catch {
    return null
  }
}

const savePortalToken = (scope: PortalTokenScope, token: string, storage?: TokenStorage) => {
  try {
    storage?.setItem(storageKey(scope), token)
  } catch {
    // storage blocked: a provider round trip then ends on the expired-session state
  }
}

const removeKeys = (keys: string[], storage: TokenStorage | undefined) => {
  try {
    for (const key of [...keys, LEGACY_STORAGE_KEY]) storage?.removeItem(key)
  } catch {
    // storage blocked: nothing was stored
  }
}

/** A token the server rejected: only that page kind's. */
export const clearPortalToken = (
  scope: PortalTokenScope | undefined,
  storage: TokenStorage | undefined = sessionStore()
) => {
  if (scope) removeKeys([storageKey(scope)], storage)
}

/** Logout: every page kind's token. */
export const clearAllPortalTokens = (storage: TokenStorage | undefined = sessionStore()) =>
  removeKeys(PORTAL_TOKEN_SCOPES.map(storageKey), storage)

/** The token a portal page of `scope` authenticates with: its URL token, kept, else the kept one. */
export const scopePortalToken = (
  scope: PortalTokenScope | undefined,
  search: string,
  storage: TokenStorage | undefined = sessionStore()
): string | null => {
  if (!scope) return null
  const urlToken = new URLSearchParams(search).get('token')
  if (urlToken) {
    savePortalToken(scope, urlToken, storage)
    return urlToken
  }
  return readPortalToken(scope, storage)
}

export const pagePortalToken = (
  location: Pick<Location, 'pathname' | 'search'>,
  storage: TokenStorage | undefined = sessionStore()
): string | null => scopePortalToken(portalTokenScope(location.pathname), location.search, storage)

type PortalClaims = { resource?: unknown; scopes?: unknown }

// Unverified: only to choose what the page offers; the server checks every call.
const portalClaims = (token: string): PortalClaims | undefined => {
  try {
    const payload = token.split('.')[1]
    if (!payload) return undefined
    const claims = JSON.parse(atob(payload.replace(/-/g, '+').replace(/_/g, '/')))
    return claims && typeof claims === 'object' ? (claims as PortalClaims) : undefined
  } catch {
    return undefined
  }
}

const RESOURCE_SCOPE: Record<string, PortalTokenScope> = {
  Customer: 'customer',
  CheckoutSession: 'checkout',
  Invoice: 'invoice',
  Quote: 'quote',
  Dashboard: 'dashboard',
}

/** The kind of resource a portal token opens. */
export const portalTokenResource = (token: string): PortalTokenScope | undefined => {
  const resource = portalClaims(token)?.resource
  if (!resource || typeof resource !== 'object') return undefined
  return RESOURCE_SCOPE[Object.keys(resource)[0]]
}

/** A customer token that may read the portal: no `scopes` (full access), or `read` or `manage`. */
export const opensCustomerPortal = (token: string): boolean => {
  if (portalTokenResource(token) !== 'customer') return false
  const scopes = portalClaims(token)?.scopes
  if (scopes === undefined || scopes === null) return true
  return Array.isArray(scopes) && scopes.some(scope => scope === 'read' || scope === 'manage')
}
