import { useEffect, useRef } from 'react'

import { buildEmbedUrl } from './url'

import type { BillingPortalOptions, EmbedView, PortalMessage } from './types'

const DEFAULT_HEIGHT = 240

const isPortalMessage = (data: unknown): data is PortalMessage =>
  typeof data === 'object' &&
  data !== null &&
  typeof (data as { type?: unknown }).type === 'string' &&
  (data as { type: string }).type.startsWith('meteroid:')

/**
 * React wrapper around the embedded billing portal.
 *
 * Renders an `<iframe>` pointing at the built URL and wires the same
 * resize/navigate postMessage protocol as the vanilla `mountBillingPortal`.
 * React is consumed via the optional peer dependency, so vanilla bundles never
 * pull it in.
 */
export const BillingPortal = ({
  token,
  baseUrl,
  view,
  theme,
  accent,
  radius,
  bg,
  surface,
  text,
  border,
  count,
  subscriptionId,
  branding,
  height = DEFAULT_HEIGHT,
  className,
  onNavigate,
}: BillingPortalOptions) => {
  const iframeRef = useRef<HTMLIFrameElement>(null)
  const src = buildEmbedUrl({
    token,
    baseUrl,
    view,
    theme,
    accent,
    radius,
    bg,
    surface,
    text,
    border,
    count,
    subscriptionId,
    branding,
    onNavigate,
  })

  useEffect(() => {
    const iframe = iframeRef.current
    if (!iframe) return
    const expectedOrigin = new URL(src).origin

    const onMessage = (event: MessageEvent) => {
      if (event.origin !== expectedOrigin) return
      if (iframe.contentWindow && event.source !== iframe.contentWindow) return
      if (!isPortalMessage(event.data)) return

      const msg = event.data
      if (msg.type === 'meteroid:resize' && typeof msg.height === 'number') {
        iframe.style.height = `${msg.height}px`
      } else if (msg.type === 'meteroid:navigate') {
        onNavigate?.(msg.target)
      }
    }

    window.addEventListener('message', onMessage)
    return () => window.removeEventListener('message', onMessage)
  }, [src, onNavigate])

  return (
    <iframe
      ref={iframeRef}
      src={src}
      title="Meteroid billing portal"
      className={className}
      allow="payment"
      style={{ width: '100%', border: 'none', display: 'block', height }}
    />
  )
}

/** Thin alias that takes `view` up front: `<BillingEmbed view="plan" .../>`. */
export const BillingEmbed = ({ view, ...rest }: BillingPortalOptions & { view: EmbedView }) => (
  <BillingPortal view={view} {...rest} />
)

export type { BillingPortalOptions, EmbedView } from './types'
