import { useEffect, useState } from 'react'

import {
  ConnectorProviderEnum,
  MandateSetupMode,
  PaymentProviderCapabilities,
} from '@/rpc/api/connectors/v1/models_pb'

export const isHostedRedirect = (caps: PaymentProviderCapabilities | undefined) =>
  caps?.mandateSetupMode === MandateSetupMode.HOSTED_REDIRECT

/** One tab per distinct connection, or per rail for a shared hosted-redirect connection (an
 *  embedded element covers all rails of one intent). The DD hosted-checkout tab fetches no intent. */
export const useRailTabs = (
  cardConnectionId: string | undefined,
  directDebitConnectionId: string | undefined,
  capabilities: PaymentProviderCapabilities | undefined,
  provider?: ConnectorProviderEnum
) => {
  const sharedConnectionId =
    cardConnectionId && cardConnectionId === directDebitConnectionId ? cardConnectionId : undefined
  const [latched, setLatched] = useState<{
    connectionId: string
    capabilities: PaymentProviderCapabilities
    provider?: ConnectorProviderEnum
  }>()
  useEffect(() => {
    if (sharedConnectionId && capabilities) {
      setLatched({ connectionId: sharedConnectionId, capabilities, provider })
    }
  }, [sharedConnectionId, capabilities, provider])

  const shared =
    sharedConnectionId && latched?.connectionId === sharedConnectionId ? latched : undefined
  const sharedCapabilities = shared?.capabilities
  const hasBoth =
    !!cardConnectionId &&
    !!directDebitConnectionId &&
    (!sharedConnectionId || isHostedRedirect(sharedCapabilities))

  return { hasBoth, sharedCapabilities, sharedProvider: shared?.provider }
}

/** Copy for the hosted-redirect button, templated on provider name and rail. */
export const hostedRedirectHelperText = (
  providerName: string,
  rail: 'card' | 'directDebit',
  afterwards: string
) =>
  `You'll be redirected to ${providerName}'s secure page to ${
    rail === 'card' ? 'enter your card details' : 'authorise a direct-debit mandate with your bank'
  }. ${afterwards}`
