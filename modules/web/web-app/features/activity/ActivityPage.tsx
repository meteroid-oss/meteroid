import { ScrollText } from 'lucide-react'

import { EditionGateCard } from '@/components/EditionGateCard'

import type { FunctionComponent } from 'react'

/**
 * The organization-wide audit log is a Meteroid Cloud / Enterprise feature.
 *
 * In the open-source edition this page renders an upgrade prompt instead of the
 * full audit trail. The per-entity activity timelines (on customers, invoices,
 * subscriptions, quotes, plans, …) remain fully functional — only this
 * org-wide aggregated view is gated.
 */
export const ActivityPage: FunctionComponent = () => {
  return (
    <EditionGateCard
      icon={<ScrollText size={24} strokeWidth={1.5} />}
      title="Audit log is not available in this edition"
      description="The organization-wide audit log — a searchable trail of every system and user action across your tenant — is part of Meteroid Cloud and Meteroid Enterprise edition."
    />
  )
}
