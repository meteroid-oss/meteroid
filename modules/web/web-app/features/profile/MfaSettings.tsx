import { ShieldCheck } from 'lucide-react'

import { EditionGateCard } from '@/components/EditionGateCard'

import type { FunctionComponent } from 'react'

/**
 * Two-factor authentication (TOTP) for user accounts is a Meteroid Cloud /
 * Enterprise feature.
 *
 * In the open-source edition this section renders an upgrade prompt instead of
 * the enrollment flow — mirroring how the org-wide audit log is gated.
 */
export const MfaSettings: FunctionComponent = () => {
  return (
    <EditionGateCard
      layout="inline"
      icon={<ShieldCheck size={24} strokeWidth={1.5} />}
      title="Two-factor authentication is not available in this edition"
      description="Protecting your account with an authenticator app (TOTP) is part of Meteroid Cloud and Meteroid Enterprise edition."
    />
  )
}
