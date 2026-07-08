/** Which view the embedded portal renders. Mirrors the `?embed=` URL param. */
export type EmbedView =
  | 'portal'
  | 'plan'
  | 'subscriptions'
  | 'subscription'
  | 'usage'
  | 'invoices'
  | 'payment-methods'

export type EmbedTheme = 'light' | 'dark'
export type EmbedRadius = 'Sharp' | 'Modern' | 'Rounded'

/**
 * Options for mounting an embedded billing portal.
 *
 * `token` is a customer-scoped portal JWT — mint it server-side via
 * `POST /api/v1/customers/{id_or_alias}/portal-token` (see the README). The
 * theme overrides map 1:1 onto the portal's URL override params.
 */
export interface BillingPortalOptions {
  /** Customer-scoped portal token (JWT). Required. Mint server-side. */
  token: string
  /** Origin hosting the portal. Defaults to `https://app.meteroid.com`. */
  baseUrl?: string
  /** Which embedded view to render. Defaults to `portal`. */
  view?: EmbedView
  /** Force the color theme. */
  theme?: EmbedTheme
  /** Hex accent color override, e.g. `#C6F94E`. */
  accent?: string
  /** Control roundness override. */
  radius?: EmbedRadius
  /**
   * Curated palette overrides (hex) to match the host product. A small, stable
   * subset of the portal's internal tokens; anything omitted is derived from
   * `accent`/`theme`. Each maps to its own URL param.
   */
  bg?: string
  surface?: string
  text?: string
  border?: string
  /** Initial iframe height in px (it auto-grows from messages). Defaults to 240. */
  height?: number
  /** Extra class name applied to the iframe element. */
  className?: string
  /** Rows per page for the compact `invoices` widget. Defaults to 5. */
  count?: number
  /** Subscription id for the single-subscription view (`view: 'subscription'`). */
  subscriptionId?: string
  /**
   * Show the "Powered by Meteroid" attribution under the widget. Defaults to
   * `true`; set `false` to hide it (maps to `?branding=false`). The tenant's
   * portal branding settings can also disable it.
   */
  branding?: boolean
  /**
   * Called when the embed asks the host to navigate, e.g. the compact `plan`
   * widget's "Manage" button posts `{ target: 'subscriptions' }`. The target is
   * a portal page (`portal` | `subscriptions` | `usage` | `invoices` | `settings`).
   *
   * When provided, the widget delegates navigation to the host instead of opening
   * the full portal in a new tab. Omit it to keep the default new-tab behavior.
   */
  onNavigate?: (target: string) => void
}

/** Messages posted by the embedded portal to the host window. */
export type PortalMessage =
  | { type: 'meteroid:resize'; height: number }
  | { type: 'meteroid:navigate'; target: string }

/** Handle returned by `mountBillingPortal`. */
export interface BillingPortalHandle {
  /** Removes the message listener and the iframe from the DOM. */
  destroy(): void
}
