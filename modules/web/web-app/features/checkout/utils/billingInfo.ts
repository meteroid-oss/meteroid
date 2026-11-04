interface BillingInformation {
  billingEmail?: string
  billingAddress?: {
    line1?: string
    city?: string
    zipCode?: string
    country?: string
  }
}

const isSet = (value?: string): boolean => !!value?.trim()

// Mirrors the server-side `Customer::has_complete_billing_information`.
export const hasCompleteBillingInformation = (customer?: BillingInformation): boolean => {
  const address = customer?.billingAddress
  return (
    isSet(customer?.billingEmail) &&
    isSet(address?.line1) &&
    isSet(address?.city) &&
    isSet(address?.zipCode) &&
    isSet(address?.country)
  )
}
