import { ShieldCheck } from 'lucide-react'

import { EditionGateCard } from '@/components/EditionGateCard'

import type { FunctionComponent } from 'react'

/**
 * Organization-wide security — MFA enforcement policy and the security activity
 * log — is a Meteroid Cloud / Enterprise feature.
 *
 * In the open-source edition this tab renders an upgrade prompt instead of the
 * full controls, mirroring how the org-wide audit log is gated.
 */
export const SecurityTab: FunctionComponent = () => {
  return (
    <EditionGateCard
      icon={<ShieldCheck size={24} strokeWidth={1.5} />}
      title="Security tab is not available in this edition"
      description="Enforcing two-factor authentication across your organization and reviewing the security activity log are part of Meteroid Cloud and Meteroid Enterprise edition."
    />
  )
}
