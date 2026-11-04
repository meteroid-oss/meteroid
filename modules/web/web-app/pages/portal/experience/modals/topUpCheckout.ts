import { portalTokenScope, readPortalToken } from '@/features/auth/portalToken'

import type { Wallet } from '@/rpc/portal/wallets/v1/models_pb'

/** Set on the portal URL a hosted top-up checkout returns to: the purchase to show. */
export const TOP_UP_RETURN_PARAM = 'topUpReturn'

/** The portal runs inside a merchant's page: leaving it would replace the merchant's embed. */
export const isEmbeddedPortal = () =>
  window.self !== window.top || new URLSearchParams(window.location.search).has('embed')

const EMBED_PARAMS = new Set(['embed', 'nav', 'count', 'sub', 'subscription'])
const CREDENTIAL_PARAM =
  /token|secret|passw|session|auth|signature|credential|api[-_]?key|^key$|^sig$|^code$/i

/**
 * Where the hosted checkout sends the customer back: the full portal's Credit page, opened on the
 * purchase stored under `purchaseKey`. It never carries the portal token or another credential:
 * the payment provider stores it, and the tab restores the token from its session on return.
 */
export const topUpReturnUrl = (href: string, purchaseKey: string) => {
  const url = new URL(href)
  url.username = ''
  url.password = ''
  url.hash = ''
  for (const name of Array.from(new Set(url.searchParams.keys()))) {
    if (EMBED_PARAMS.has(name) || CREDENTIAL_PARAM.test(name)) url.searchParams.delete(name)
  }
  url.searchParams.set('page', 'balance')
  url.searchParams.set(TOP_UP_RETURN_PARAM, purchaseKey)
  return url.toString()
}

/** The checkout return URL with the token the tab kept; null when there is nothing to restore. */
export const withStoredPortalToken = (href: string, storedToken: string | null): string | null => {
  const url = new URL(href)
  if (!url.searchParams.has(TOP_UP_RETURN_PARAM) || url.searchParams.has('token')) return null
  if (!storedToken) return null
  url.searchParams.set('token', storedToken)
  return url.toString()
}

// Without a stored token the URL stays as it is and the portal shows its expired-session state.
const restorePortalTokenAfterCheckout = () => {
  if (typeof window === 'undefined') return
  const stored = readPortalToken(portalTokenScope(window.location.pathname))
  const restored = withStoredPortalToken(window.location.href, stored)
  if (restored) window.history.replaceState(window.history.state, '', restored)
}

// On import, so before the router and usePortalToken() first read the location.
restorePortalTokenAfterCheckout()

/**
 * A purchase that opens the customer's money wallet has no wallet id until it is sent: it is
 * stored, and returned to, under its currency.
 */
export const openingPurchaseKey = (currency: string) => `new-${currency.trim().toUpperCase()}`

/** The wallet a checkout return names: by id, else the money wallet an opening purchase opened. */
export const returnedWallet = <W extends Pick<Wallet, 'id' | 'currency' | 'creditUnit'>>(
  purchaseKey: string | null,
  wallets: W[]
): W | undefined =>
  purchaseKey
    ? (wallets.find(wallet => wallet.id === purchaseKey) ??
      wallets.find(
        wallet => !wallet.creditUnit && openingPurchaseKey(wallet.currency) === purchaseKey
      ))
    : undefined

/**
 * Whether a settled top-up's credit shows on the wallet: its credit on account grew by the `bought`
 * credit from `before` and the live balance caught up with it. The purchase is followed under its
 * wallet, else its currency.
 */
export const topUpCreditLanded = (
  wallets: Pick<Wallet, 'id' | 'currency' | 'creditUnit' | 'available' | 'liveStale'>[],
  purchaseKey: string,
  before: bigint,
  bought: bigint
): boolean => {
  const wallet = returnedWallet(purchaseKey, wallets)
  return !!wallet && wallet.available - before >= bought && !wallet.liveStale
}
