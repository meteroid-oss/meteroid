import { paymentProvider } from '@/features/payments/providers'
import { type Connector } from '@/rpc/api/connectors/v1/models_pb'

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
 * Advisory checks mirroring provider-side requirements (declared per provider in the
 * registry), so staff can fix a customer before a payment fails. Not enforced.
 */
export const getPaymentRoutingWarnings = (
  customer: Pick<Customer, 'billingAddress'>,
  providers: Providers
): PaymentRoutingWarning[] => {
  const warnings: PaymentRoutingWarning[] = []

  const addr = customer.billingAddress
  const address = {
    hasAnyField: Boolean(
      addr && (addr.line1 || addr.line2 || addr.city || addr.state || addr.zipCode)
    ),
    missingCountry: !addr?.country,
  }

  const check = (rail: PaymentRoutingWarning['rail'], connector?: Connector) => {
    if (!connector) return
    const message = paymentProvider(connector.provider)?.customerAddressWarning?.(address)
    if (message) {
      warnings.push({ rail, providerAlias: connector.alias, message })
    }
  }

  check('Card', providers.cardProvider)
  check('Direct debit', providers.directDebitProvider)

  return warnings
}
