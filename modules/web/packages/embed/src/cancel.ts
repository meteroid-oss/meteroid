import { buildEmbedUrl } from './url'

import type { CancelFlowOptions, CancelFlowOutcome, CancelFlowResult, PortalMessage } from './types'

const OUTCOMES: readonly CancelFlowOutcome[] = ['saved', 'contact_requested', 'cancelled', 'closed']

/**
 * Opens the cancel flow of a subscription over the page, as the portal runs it: the tenant's
 * reasons and offers when Retention is on, the plain confirmation otherwise. Resolves once the
 * customer is done, with how it ended; the overlay is removed by then.
 */
export const openCancelFlow = (opts: CancelFlowOptions): Promise<CancelFlowResult> =>
  new Promise(resolve => {
    const src = buildEmbedUrl({ ...opts, view: 'cancel' })
    const expectedOrigin = new URL(src).origin

    const iframe = document.createElement('iframe')
    iframe.src = src
    iframe.title = 'Cancel subscription'
    iframe.setAttribute('allow', 'payment')
    Object.assign(iframe.style, {
      position: 'fixed',
      inset: '0',
      width: '100%',
      height: '100%',
      border: 'none',
      background: 'transparent',
      colorScheme: 'normal',
      zIndex: '2147483647',
    })

    const previousOverflow = document.body.style.overflow
    const finish = (outcome: CancelFlowOutcome, effectiveDate?: string) => {
      window.removeEventListener('message', onMessage)
      document.body.style.overflow = previousOverflow
      iframe.remove()
      resolve({ outcome, effectiveDate })
    }

    const onMessage = (event: MessageEvent) => {
      if (event.origin !== expectedOrigin) return
      if (iframe.contentWindow && event.source !== iframe.contentWindow) return
      const msg = event.data as PortalMessage | null
      if (msg?.type !== 'meteroid:cancel-flow') return
      const outcome = OUTCOMES.includes(msg.outcome) ? msg.outcome : 'closed'
      finish(outcome, typeof msg.effectiveDate === 'string' ? msg.effectiveDate : undefined)
    }

    window.addEventListener('message', onMessage)
    document.body.style.overflow = 'hidden'
    document.body.appendChild(iframe)
  })
