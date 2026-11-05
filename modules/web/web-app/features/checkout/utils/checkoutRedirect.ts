import { hostedReturnTarget } from '@/features/checkout/utils/hostedReturn'
import { safeRedirectUrl } from '@/lib/utils/url'

/**
 * Merchant-set redirect targets (`success_url` / `cancel_url` on the checkout session), read from
 * the token-authenticated GetCheckout. They travel to the success page through router state and a
 * same-origin stash — never through the URL, so a crafted link cannot turn the success page into
 * an open redirect. The stash covers the hosted-provider round trip where the webhook completes
 * the session before the customer is back and GetCheckout no longer answers.
 */

export interface CheckoutRedirect {
  successUrl?: string
  cancelUrl?: string
}

// Absolute: relative to `/checkout/:subscriptionId` it would name no route.
export const CHECKOUT_SUCCESS_PATH = '/checkout/success'

export interface CheckoutSuccessState {
  returnUrl?: string
  /** A link back, without the redirect `returnUrl` makes. */
  backTo?: { url: string; label: string }
}

const STASH_KEY = 'checkout_redirect'

export const stashCheckoutRedirect = (redirect: CheckoutRedirect): void => {
  if (typeof window === 'undefined') return
  try {
    if (redirect.successUrl || redirect.cancelUrl) {
      sessionStorage.setItem(STASH_KEY, JSON.stringify(redirect))
    } else {
      sessionStorage.removeItem(STASH_KEY)
    }
  } catch {
    // storage blocked: the customer stays on the confirmation page
  }
}

export const consumeCheckoutRedirect = (): CheckoutRedirect => {
  if (typeof window === 'undefined') return {}
  try {
    const raw = sessionStorage.getItem(STASH_KEY)
    sessionStorage.removeItem(STASH_KEY)
    return raw ? (JSON.parse(raw) as CheckoutRedirect) : {}
  } catch {
    return {}
  }
}

/**
 * Pre-`success_url` integrations (and go-live, until it sets it through the SDK) append
 * `?return_url=` to the checkout URL. Honoured only when the session sets no `success_url`, and
 * only from the token-gated checkout page.
 */
export const legacyReturnUrl = (searchParams: URLSearchParams): string | undefined =>
  safeRedirectUrl(searchParams.get('return_url'))

/**
 * A portal page's checkout (plan change, add-on, seats): it returns to `returnFrom` without its
 * token, which the returned page restores from the tab, so the checkout never carries it.
 */
export const portalCheckoutHref = (checkoutToken: string, returnFrom: string): string => {
  const params = new URLSearchParams({
    token: checkoutToken,
    return_url: hostedReturnTarget(returnFrom),
  })
  return `/checkout?${params.toString()}`
}
