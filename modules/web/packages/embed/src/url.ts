import type { BillingPortalOptions } from './types'

export const DEFAULT_BASE_URL = 'https://app.meteroid.com'

/**
 * Build the iframe `src` URL for an embedded portal.
 *
 * Only the overrides that were actually provided are appended. The `token` and
 * `accent` values are URL-encoded; the portal reads them from the query string.
 */
export const buildEmbedUrl = (opts: BillingPortalOptions): string => {
  const base = (opts.baseUrl || DEFAULT_BASE_URL).replace(/\/+$/, '')
  const params = new URLSearchParams()

  params.set('token', opts.token)
  params.set('embed', opts.view ?? 'portal')
  if (opts.theme) params.set('theme', opts.theme)
  if (opts.accent) params.set('accent', opts.accent)
  if (opts.radius) params.set('radius', opts.radius)
  // Curated palette overrides — only emitted when provided.
  if (opts.bg) params.set('bg', opts.bg)
  if (opts.surface) params.set('surface', opts.surface)
  if (opts.text) params.set('text', opts.text)
  if (opts.border) params.set('border', opts.border)
  if (opts.count != null) params.set('count', String(opts.count))
  if (opts.subscriptionId) params.set('subscription', opts.subscriptionId)
  // Default-on; only emit the param when the host opts out.
  if (opts.branding === false) params.set('branding', 'false')
  // Tell the widget the host handles navigation (it will postMessage instead of
  // opening the full portal itself).
  if (opts.onNavigate) params.set('nav', 'host')

  // URLSearchParams already percent-encodes both keys and values (incl. the
  // `#` in accent and any JWT padding in the token).
  return `${base}/portal/customer?${params.toString()}`
}
