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
