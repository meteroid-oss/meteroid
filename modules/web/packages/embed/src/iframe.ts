import { buildEmbedUrl } from './url'

import type { BillingPortalHandle, BillingPortalOptions, PortalMessage } from './types'

const DEFAULT_HEIGHT = 240

const resolveTarget = (target: string | HTMLElement): HTMLElement => {
  if (typeof target === 'string') {
    const el = document.querySelector(target)
    if (!el) throw new Error(`[portal-embed] mount target not found: ${target}`)
    return el as HTMLElement
  }
  return target
}

const isPortalMessage = (data: unknown): data is PortalMessage =>
  typeof data === 'object' &&
  data !== null &&
  typeof (data as { type?: unknown }).type === 'string' &&
  (data as { type: string }).type.startsWith('meteroid:')

/**
 * Mount an embedded billing portal into `target` as an iframe.
 *
 * Listens for `meteroid:resize` (auto-grows the iframe) and `meteroid:navigate`
 * (forwarded to `opts.onNavigate`). Messages are validated against the iframe's
 * own origin. Call `destroy()` to tear everything down.
 */
export const mountBillingPortal = (
  target: string | HTMLElement,
  opts: BillingPortalOptions
): BillingPortalHandle => {
  const container = resolveTarget(target)
  const src = buildEmbedUrl(opts)
  const expectedOrigin = new URL(src).origin

  const iframe = document.createElement('iframe')
  iframe.src = src
  iframe.title = 'Meteroid billing portal'
  iframe.style.width = '100%'
  iframe.style.border = 'none'
  iframe.style.display = 'block'
  iframe.style.height = `${opts.height ?? DEFAULT_HEIGHT}px`
  iframe.setAttribute('allow', 'payment')
  if (opts.className) iframe.className = opts.className

  const onMessage = (event: MessageEvent) => {
    // Only trust messages from the portal origin and from this iframe's window.
    if (event.origin !== expectedOrigin) return
    if (iframe.contentWindow && event.source !== iframe.contentWindow) return
    if (!isPortalMessage(event.data)) return

    const msg = event.data
    if (msg.type === 'meteroid:resize' && typeof msg.height === 'number') {
      iframe.style.height = `${msg.height}px`
    } else if (msg.type === 'meteroid:navigate') {
      opts.onNavigate?.(msg.target)
    }
  }

  window.addEventListener('message', onMessage)
  container.appendChild(iframe)

  return {
    destroy() {
      window.removeEventListener('message', onMessage)
      iframe.remove()
    },
  }
}
