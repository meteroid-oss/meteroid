import { createConnectQueryKey, useMutation } from '@connectrpc/connect-query'
import { useQueryClient } from '@tanstack/react-query'
import { useCallback, useMemo } from 'react'

import { useQuery } from '@/lib/connectrpc'
import {
  getCustomerPortalOverview,
  listInvoices,
} from '@/rpc/portal/customer/v1/customer-PortalCustomerService_connectquery'
import {
  cancelScheduledEvent,
  cancelSubscription,
  confirmPlanChange,
  getSubscriptionDetails,
  getUpcomingInvoice,
  listAdjustableComponents,
  listAvailableAddOns,
  listAvailablePlans,
  previewPlanChange,
  purchaseAddOn,
  updateAddOnQuantity,
  updateSeats,
} from '@/rpc/portal/subscription/v1/subscription-PortalSubscriptionService_connectquery'
import {
  AddOnPurchaseStatus,
  AdjustmentStatus,
  PlanChangeStatus,
} from '@/rpc/portal/subscription/v1/subscription_pb'

import { PortalBranding, PortalRoundness, PortalThemeMode } from './theme'

import type { CustomerPortalOverview } from '@/rpc/portal/customer/v1/models_pb'
import type { DescMethodUnary } from '@bufbuild/protobuf'

const asThemeMode = (v?: string): PortalThemeMode | undefined =>
  v === 'light' || v === 'dark' ? v : undefined
const asRoundness = (v?: string): PortalRoundness | undefined =>
  v === 'Sharp' || v === 'Modern' || v === 'Rounded' ? v : undefined

/** Read the portal token from the current URL (used to build cross-page links). */
export const usePortalToken = (): string => {
  return useMemo(() => new URLSearchParams(window.location.search).get('token') ?? '', [])
}

/** Map the invoicing-entity fields on the overview to portal branding. */
export const brandingFromOverview = (overview?: CustomerPortalOverview): PortalBranding => ({
  companyName: overview?.invoicingEntityName,
  logoUrl: overview?.invoicingEntityLogoUrl,
  accent: overview?.invoicingEntityBrandColor,
  theme: asThemeMode(overview?.invoicingEntityThemeMode),
  roundness: asRoundness(overview?.invoicingEntityRoundness),
})

/**
 * Every read query the portal renders. After any mutation we invalidate the
 * whole set so all screens (overview/subscription list, detail, usage,
 * invoices) reflect the new state — not just the screen that triggered it.
 */
const PORTAL_QUERY_SCHEMAS: DescMethodUnary[] = [
  getCustomerPortalOverview,
  listInvoices,
  getSubscriptionDetails,
  listAvailablePlans,
  previewPlanChange,
  listAvailableAddOns,
  listAdjustableComponents,
  getUpcomingInvoice,
]

/** Invalidate all portal caches. Call after any mutating portal action. */
export const useInvalidatePortal = () => {
  const queryClient = useQueryClient()
  return useCallback(
    () =>
      Promise.all(
        PORTAL_QUERY_SCHEMAS.map(schema =>
          queryClient.invalidateQueries({
            queryKey: createConnectQueryKey({ schema, cardinality: undefined }),
          })
        )
      ),
    [queryClient]
  )
}

export const usePortalOverview = () => {
  const query = useQuery(getCustomerPortalOverview)
  const branding = useMemo(() => brandingFromOverview(query.data?.overview), [query.data])
  return { ...query, overview: query.data?.overview, branding, refetch: query.refetch }
}

export const useInvoices = (page: number, perPage = 8) =>
  useQuery(listInvoices, { pagination: { page, perPage } })

/**
 * Everything the subscription-detail screen and change-plan modal need:
 * details, available plans, a live proration preview, add-ons, the upcoming
 * invoice, and the three mutating actions with checkout-redirect handling.
 */
export const useSubscription = (
  subscriptionId: string,
  opts: { selectedPlanVersionId?: string; loadPlans?: boolean } = {}
) => {
  const token = usePortalToken()

  const details = useQuery(getSubscriptionDetails, { subscriptionId })
  const plans = useQuery(
    listAvailablePlans,
    { subscriptionId },
    { enabled: !!opts.loadPlans }
  )
  const preview = useQuery(
    previewPlanChange,
    { subscriptionId, newPlanVersionId: opts.selectedPlanVersionId ?? '' },
    { enabled: !!opts.selectedPlanVersionId }
  )
  const addOns = useQuery(listAvailableAddOns, { subscriptionId })
  const adjustable = useQuery(listAdjustableComponents, { subscriptionId })
  const upcoming = useQuery(
    getUpcomingInvoice,
    { subscriptionId },
    { enabled: !!details.data?.subscription }
  )

  const confirmMutation = useMutation(confirmPlanChange)
  const purchaseMutation = useMutation(purchaseAddOn)
  const cancelMutation = useMutation(cancelScheduledEvent)
  const cancelSubMutation = useMutation(cancelSubscription)
  const addOnQtyMutation = useMutation(updateAddOnQuantity)
  const seatsMutation = useMutation(updateSeats)

  const invalidatePortal = useInvalidatePortal()

  const goToCheckout = useCallback(
    (checkoutToken: string) => {
      // Carry the current portal URL (which holds the portal `token`) so the
      // success page can send the customer straight back here after paying.
      const params = new URLSearchParams({
        token: checkoutToken,
        return_url: window.location.href,
      })
      window.location.href = `/portal/checkout?${params.toString()}`
    },
    []
  )

  /** Confirm a plan change. Redirects to checkout when payment is required. */
  const confirmPlan = useCallback(
    async (newPlanVersionId: string) => {
      const res = await confirmMutation.mutateAsync({ subscriptionId, newPlanVersionId })
      if (res.status === PlanChangeStatus.PLAN_CHANGE_CHECKOUT_REQUIRED && res.checkoutToken) {
        goToCheckout(res.checkoutToken)
        return { redirected: true as const, res }
      }
      await invalidatePortal()
      return { redirected: false as const, res }
    },
    [confirmMutation, subscriptionId, goToCheckout, invalidatePortal]
  )

  /** Purchase an add-on. Redirects to checkout when payment is required. */
  const purchase = useCallback(
    async (addOnId: string) => {
      const res = await purchaseMutation.mutateAsync({ subscriptionId, addOnId })
      if (res.status === AddOnPurchaseStatus.ADDON_PURCHASE_CHECKOUT_REQUIRED && res.checkoutToken) {
        goToCheckout(res.checkoutToken)
        return { redirected: true as const, res }
      }
      await invalidatePortal()
      return { redirected: false as const, res }
    },
    [purchaseMutation, subscriptionId, goToCheckout, invalidatePortal]
  )

  const cancelScheduled = useCallback(
    async (eventId: string) => {
      await cancelMutation.mutateAsync({ subscriptionId, eventId })
      await invalidatePortal()
    },
    [cancelMutation, subscriptionId, invalidatePortal]
  )

  /**
   * Set an add-on's quantity (0 = remove). Increases apply immediately
   * (prorated); decreases/removals are scheduled for period end. Redirects to
   * checkout only when the backend requires upfront payment.
   */
  const setAddOnQuantity = useCallback(
    async (addOnId: string, quantity: number) => {
      const res = await addOnQtyMutation.mutateAsync({ subscriptionId, addOnId, quantity })
      if (res.status === AdjustmentStatus.ADJUSTMENT_CHECKOUT_REQUIRED && res.checkoutToken) {
        goToCheckout(res.checkoutToken)
        return { redirected: true as const, res }
      }
      await invalidatePortal()
      return { redirected: false as const, res }
    },
    [addOnQtyMutation, subscriptionId, goToCheckout, invalidatePortal]
  )

  /** Set a seat/slot component to a target quantity. */
  const setSeats = useCallback(
    async (componentId: string, quantity: number) => {
      const res = await seatsMutation.mutateAsync({ subscriptionId, componentId, quantity })
      if (res.status === AdjustmentStatus.ADJUSTMENT_CHECKOUT_REQUIRED && res.checkoutToken) {
        goToCheckout(res.checkoutToken)
        return { redirected: true as const, res }
      }
      await invalidatePortal()
      return { redirected: false as const, res }
    },
    [seatsMutation, subscriptionId, goToCheckout, invalidatePortal]
  )

  /** Cancel the subscription (end-of-period by default, or immediately). */
  const cancelSub = useCallback(
    async (opts: { immediate?: boolean; reason?: string } = {}) => {
      const res = await cancelSubMutation.mutateAsync({
        subscriptionId,
        immediate: opts.immediate ?? false,
        reason: opts.reason,
      })
      await invalidatePortal()
      return res
    },
    [cancelSubMutation, subscriptionId, invalidatePortal]
  )

  return {
    token,
    details,
    subscription: details.data?.subscription,
    plans,
    preview: preview.data?.preview,
    previewPlanName: preview.data?.newPlanName,
    isPreviewLoading: preview.isFetching,
    addOns: addOns.data?.addOns ?? [],
    adjustableComponents: adjustable.data?.components ?? [],
    canSelfServe: adjustable.data?.canSelfServe ?? false,
    allowDowngrade: adjustable.data?.allowDowngrade ?? true,
    upcomingInvoice: upcoming.data?.invoice,
    isUpcomingLoading: upcoming.isLoading,
    invalidatePortal,
    setAddOnQuantity,
    addOnQtyPending: addOnQtyMutation.isPending,
    setSeats,
    seatsPending: seatsMutation.isPending,
    confirmPlan,
    confirmPending: confirmMutation.isPending,
    purchase,
    purchasePending: purchaseMutation.isPending,
    cancelScheduled,
    cancelPending: cancelMutation.isPending,
    cancelSub,
    cancelSubPending: cancelSubMutation.isPending,
  }
}
