import { ConnectorProviderEnum, type Connector } from '@/rpc/api/connectors/v1/models_pb'

import type { Customer } from '@/rpc/api/customers/v1/models_pb'

export interface PaymentRoutingWarning {
  rail: 'Card' | 'Direct debit'
  providerAlias: string
  message: string
}

interface Providers {
  cardProvider?: Connector
  directDebitProvider?: Connector
}

/**
 * Advisory checks that mirror provider-side requirements, so staff can fix a
 * customer before a real payment fails. Frontend heuristic, not enforcement —
 * the payment path keeps its own validation.
 */
export const getPaymentRoutingWarnings = (
  customer: Pick<Customer, 'billingAddress'>,
  providers: Providers
): PaymentRoutingWarning[] => {
  const warnings: PaymentRoutingWarning[] = []

  const addr = customer.billingAddress
  // GoCardless rejects customer/mandate creation with "country_code is required
  // if any address fields are provided" — so a partial address is the trap.
  const hasAnyAddressField = Boolean(
    addr && (addr.line1 || addr.line2 || addr.city || addr.state || addr.zipCode)
  )
  const missingCountry = !addr?.country

  const check = (rail: PaymentRoutingWarning['rail'], connector?: Connector) => {
    if (!connector) return
    if (
      connector.provider === ConnectorProviderEnum.GOCARDLESS &&
      hasAnyAddressField &&
      missingCountry
    ) {
      warnings.push({
        rail,
        providerAlias: connector.alias,
        message: `Add the customer's country. GoCardless requires it once an address is set, otherwise ${rail.toLowerCase()} setup fails.`,
      })
    }
  }

  check('Card', providers.cardProvider)
  check('Direct debit', providers.directDebitProvider)

  return warnings
}
