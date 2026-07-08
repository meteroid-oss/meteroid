import { createContext, useContext } from 'react'

import type { CustomerPortalOverview } from '@/rpc/portal/customer/v1/models_pb'

export type PortalTab = 'overview' | 'subscriptions' | 'usage' | 'invoices' | 'settings'

export const PORTAL_TABS: { key: PortalTab; label: string }[] = [
  { key: 'overview', label: 'Overview' },
  { key: 'subscriptions', label: 'Subscriptions' },
  { key: 'usage', label: 'Usage' },
  { key: 'invoices', label: 'Invoices' },
  { key: 'settings', label: 'Settings' },
]

export interface PortalNav {
  /** The loaded customer overview (subscriptions, payment methods, customer…). */
  overview: CustomerPortalOverview
  /** Portal token from the URL, for building cross-page links (checkout, PDFs). */
  token: string
  /** Primary display currency for the customer. */
  currency: string
  /** Switch the top-level tab. */
  goTo: (tab: PortalTab) => void
  /** Open the subscription-detail screen for an id. */
  openSubscription: (subscriptionId: string) => void
  /** Open the change-plan modal for a subscription. */
  openChangePlan: (subscriptionId: string) => void
  /** Re-fetch the overview after a mutation. */
  refetchOverview: () => void
}

export const PortalNavContext = createContext<PortalNav | null>(null)

export const usePortalNav = (): PortalNav => {
  const ctx = useContext(PortalNavContext)
  if (!ctx) throw new Error('usePortalNav must be used within the portal shell')
  return ctx
}
