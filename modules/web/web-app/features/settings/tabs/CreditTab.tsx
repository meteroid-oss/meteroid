import { FunctionComponent } from 'react'

import { PortalCreditSettings } from '@/features/settings/components/PortalCreditSettings'
import { SettingsSection } from '@/features/settings/components/SettingsSection'
import { CreditUnitsCard } from '@/features/settings/components/creditunits'

export const CreditTab: FunctionComponent = () => (
  <div className="flex max-w-3xl flex-col gap-6">
    <PortalCreditSettings />
    <SettingsSection
      title="Credit units"
      description="Denominations to price or grant usage in, instead of money."
    >
      <CreditUnitsCard />
    </SettingsSection>
  </div>
)

export default CreditTab
