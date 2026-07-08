import { buildEmbedUrl, DEFAULT_BASE_URL } from './url'
import { mountBillingPortal } from './iframe'

import type { BillingPortalOptions, EmbedView } from './types'

/**
 * Browser-global (IIFE) entry. Exposes the vanilla API on `window.Meteroid`
 * and auto-mounts any declaratively-configured embeds so a plain `<script>`
 * tag works without writing any JS.
 *
 * This module is intentionally the only place that touches `window` and the
 * DOM at import time — the ESM/CJS builds stay side-effect-free.
 */

const api = { mountBillingPortal, buildEmbedUrl, DEFAULT_BASE_URL }

type MeteroidGlobal = typeof api

declare global {
  interface Window {
    Meteroid?: MeteroidGlobal
  }
}

/**
 * Build `BillingPortalOptions` from an element's `data-*` attributes.
 * Returns `null` when there is no token (nothing to mount).
 */
const optionsFromDataset = (el: HTMLElement): BillingPortalOptions | null => {
  const token = el.getAttribute('data-token') ?? el.getAttribute('data-meteroid-token')
  if (!token) return null

  const opts: BillingPortalOptions = { token }

  const view = el.getAttribute('data-view')
  if (view) opts.view = view as EmbedView

  const theme = el.getAttribute('data-theme')
  if (theme) opts.theme = theme as BillingPortalOptions['theme']

  const accent = el.getAttribute('data-accent')
  if (accent) opts.accent = accent

  const radius = el.getAttribute('data-radius')
  if (radius) opts.radius = radius as BillingPortalOptions['radius']

  const bg = el.getAttribute('data-bg')
  if (bg) opts.bg = bg
  const surface = el.getAttribute('data-surface')
  if (surface) opts.surface = surface
  const text = el.getAttribute('data-text')
  if (text) opts.text = text
  const border = el.getAttribute('data-border')
  if (border) opts.border = border

  const baseUrl = el.getAttribute('data-base-url')
  if (baseUrl) opts.baseUrl = baseUrl

  const count = el.getAttribute('data-count')
  if (count) opts.count = Number(count)

  const subscriptionId = el.getAttribute('data-subscription-id')
  if (subscriptionId) opts.subscriptionId = subscriptionId

  if (el.getAttribute('data-branding') === 'false') opts.branding = false

  return opts
}

/**
 * Resolve where a declaratively-configured embed should mount.
 *
 * A `<script data-meteroid-portal>` tag mounts into a freshly-inserted `<div>`
 * placed immediately after the script; any other element mounts into itself.
 */
const resolveMountPoint = (el: HTMLElement): HTMLElement => {
  if (el.tagName === 'SCRIPT') {
    const host = document.createElement('div')
    el.parentNode?.insertBefore(host, el.nextSibling)
    return host
  }
  return el
}

/** Scan the document for declaratively-configured embeds and mount them. */
const autoMount = (): void => {
  const nodes = document.querySelectorAll<HTMLElement>(
    '[data-meteroid-portal],[data-meteroid-token]'
  )

  nodes.forEach(el => {
    // Don't double-mount if the script re-runs.
    if (el.getAttribute('data-meteroid-mounted') === 'true') return

    const opts = optionsFromDataset(el)
    if (!opts) return

    el.setAttribute('data-meteroid-mounted', 'true')
    mountBillingPortal(resolveMountPoint(el), opts)
  })
}

// Expose the programmatic API regardless of how/when this bundle loads.
if (typeof window !== 'undefined') {
  window.Meteroid = api

  if (typeof document !== 'undefined') {
    if (document.readyState === 'loading') {
      document.addEventListener('DOMContentLoaded', autoMount)
    } else {
      autoMount()
    }
  }
}

export { mountBillingPortal, buildEmbedUrl, DEFAULT_BASE_URL }
