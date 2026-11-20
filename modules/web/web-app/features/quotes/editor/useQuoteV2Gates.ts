import { entitlementCodes, useEntitlement } from '@/hooks/useEntitlement'

import type { V2Presence } from '@/features/quotes/editor/quoteV2'

/** Each v2 control shows when the tenant has it, or when the quote already uses it. */
export const useQuoteV2Gates = (present: V2Presence) => {
  const full = useEntitlement(entitlementCodes.QUOTES_FULL)

  return {
    lineEditing: full || present.lineEditing,
    verified: full || present.verified,
    countersigners: full || present.countersigners,
    hubspot: full || present.hubspot,
    customerActions: full || present.customerActions,
  }
}
