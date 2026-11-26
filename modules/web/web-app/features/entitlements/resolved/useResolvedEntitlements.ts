import { skipToken } from '@connectrpc/connect-query'

import { useQuery } from '@/lib/connectrpc'
import {
  getResolvedEntitlementsForAddOn,
  getResolvedEntitlementsForPlanVersion,
  getResolvedEntitlementsForProduct,
  getResolvedEntitlementsForQuote,
  getResolvedEntitlementsForSelection,
  getResolvedEntitlementsForSubscription,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'

// ── Entity types ──────────────────────────────────────────────────────────────

/**
 * A persisted entity that can have resolved entitlements fetched via gRPC.
 * These map 1-to-1 with the `getResolvedEntitlementsFor*` RPCs.
 */
export type PersistedEntity =
  | { type: 'product'; id: string }
  | { type: 'add-on'; id: string }
  | { type: 'plan-version'; id: string }
  | { type: 'subscription'; id: string }
  | { type: 'quote'; id: string }

/**
 * In-flight selection (plan version + add-ons) used during entity creation.
 * Not a persisted entity — no id.
 */
export type SelectionInput = {
  planVersionId: string
  addOnIds: string[]
  extraProductIds?: string[]
  /** Product IDs whose entitlements should be excluded (e.g. from removed price components). Client-side filter only. */
  removedProductIds?: string[]
}

// ── Query hooks ───────────────────────────────────────────────────────────────

/**
 * Fetches resolved entitlements (with origin) for any persisted entity.
 * Uses `skipToken` for the branches that don't match the active type so
 * connect-query skips the inactive RPCs (React hook rules respected).
 */
export const useResolvedEntitlementsForEntity = (entity: PersistedEntity) => {
  const productQuery = useQuery(
    getResolvedEntitlementsForProduct,
    entity.type === 'product' ? { productId: entity.id } : skipToken
  )
  const addOnQuery = useQuery(
    getResolvedEntitlementsForAddOn,
    entity.type === 'add-on' ? { addOnId: entity.id } : skipToken
  )
  const planVersionQuery = useQuery(
    getResolvedEntitlementsForPlanVersion,
    entity.type === 'plan-version' ? { planVersionId: entity.id } : skipToken
  )
  const subscriptionQuery = useQuery(
    getResolvedEntitlementsForSubscription,
    entity.type === 'subscription' ? { subscriptionId: entity.id } : skipToken
  )
  const quoteQuery = useQuery(
    getResolvedEntitlementsForQuote,
    entity.type === 'quote' ? { quoteId: entity.id } : skipToken
  )

  switch (entity.type) {
    case 'product':
      return {
        ...productQuery,
        entitlements: productQuery.data?.entitlements ?? [],
      }
    case 'add-on':
      return {
        ...addOnQuery,
        entitlements: addOnQuery.data?.entitlements ?? [],
      }
    case 'plan-version':
      return {
        ...planVersionQuery,
        entitlements: planVersionQuery.data?.entitlements ?? [],
      }
    case 'subscription':
      return {
        ...subscriptionQuery,
        entitlements: subscriptionQuery.data?.entitlements ?? [],
      }
    case 'quote':
      return {
        ...quoteQuery,
        entitlements: quoteQuery.data?.entitlements ?? [],
      }
  }
}

/**
 * Fetches resolved entitlements for an in-flight selection (plan version + add-ons).
 * Used during entity creation before the entity is persisted.
 */
export const useResolvedEntitlementsForSelection = (input: SelectionInput) => {
  const selectionQuery = useQuery(getResolvedEntitlementsForSelection, {
    planVersionId: input.planVersionId,
    addOnIds: input.addOnIds,
    extraProductIds: input.extraProductIds ?? [],
  })
  const removedSet = new Set(input.removedProductIds ?? [])
  const entitlements = (selectionQuery.data?.entitlements ?? []).filter(
    e => !e.feature?.product?.id || !removedSet.has(e.feature.product.id)
  )
  return {
    ...selectionQuery,
    entitlements,
  }
}
