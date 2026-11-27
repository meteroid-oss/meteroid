import { FunctionComponent } from 'react'

import { PortalCreditSettings } from '@/features/settings/components/PortalCreditSettings'
import { CreditUnitsCard } from '@/features/settings/components/creditunits'
import { entitlementCodes, useEntitlement } from '@/hooks/useEntitlement'

export const CreditTab: FunctionComponent = () => {
  const creditPricing = useEntitlement(entitlementCodes.CREDIT_PRICING)
  return (
    <div className="flex max-w-3xl flex-col gap-6">
      <PortalCreditSettings />
      {creditPricing && <CreditUnitsCard />}
    </div>
  )
}

export default CreditTab
