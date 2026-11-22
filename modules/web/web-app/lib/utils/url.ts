const REDIRECT_PROTOCOLS = new Set(['http:', 'https:'])

/**
 * Only allows http and https links. Anything else, like `javascript:`, could run code on our
 * page. Relative links are resolved against the current page.
 */
export const safeRedirectUrl = (value: string | null | undefined): string | undefined => {
  if (!value) return undefined
  try {
    const base = typeof window === 'undefined' ? 'http://localhost' : window.location.href
    const url = new URL(value, base)
    return REDIRECT_PROTOCOLS.has(url.protocol) ? url.href : undefined
  } catch {
    return undefined
  }
}

/**
 * Turns a `returnUrl` into an in-app path, or `undefined` when it would leave the app: the
 * value is resolved the way the router will, and must stay on this origin.
 */
export const sameOriginPath = (value: string | null | undefined): string | undefined => {
  if (!value || !value.startsWith('/')) return undefined
  try {
    const origin = typeof window === 'undefined' ? 'http://localhost' : window.location.origin
    const url = new URL(value, origin)
    if (url.origin !== origin || url.pathname.startsWith('//')) return undefined
    return `${url.pathname}${url.search}${url.hash}`
  } catch {
    return undefined
  }
}
