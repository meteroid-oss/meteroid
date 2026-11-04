import { EntityEntitlementsSection } from '@/features/entitlements/EntityEntitlementsSection'

interface Props {
  customerId: string
}

/**
 * Customer-level entitlement overrides — a top-priority force. Grants set here apply across the
 * whole account and override whatever a plan or subscription sets. This is how you grant or deny
 * a feature for a customer regardless of (or in the absence of) a subscription — comps, design
 * partners, manual provisioning. They win everywhere for this customer, so use them sparingly.
 */
export const CustomerEntitlementOverrides = ({ customerId }: Props) => (
  <EntityEntitlementsSection
    entity={{ EntityId: { case: 'customerId', value: customerId } }}
    title="Customer overrides"
    emptyText="No overrides. The plan's entitlements apply."
    hint="Account-wide overrides for this customer. They win over plan- and subscription-level entitlements and apply to every subscription (and to customers with no subscription). Use them to force a feature on or off for the whole account, regardless of what the customer is subscribed to."
  />
)
