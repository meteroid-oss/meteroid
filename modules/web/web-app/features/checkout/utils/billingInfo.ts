import { Customer } from '@/rpc/api/customers/v1/models_pb'

const isSet = (value: string | undefined): boolean => !!value && value.trim().length > 0

// Mirrors the server-side `Customer::has_complete_billing_information`.
export const hasCompleteBillingInformation = (customer?: Customer): boolean => {
  if (!customer) return false

  const hasEmail = isSet(customer.billingEmail)
  const address = customer.billingAddress
  const hasAddress =
    !!address &&
    isSet(address.line1) &&
    isSet(address.city) &&
    isSet(address.zipCode) &&
    isSet(address.country)

  return hasEmail && hasAddress
}
