import { create, type MessageInitShape } from '@bufbuild/protobuf';
import { createConnectQueryKey, skipToken, useMutation } from '@connectrpc/connect-query';
import { useQueryClient } from '@tanstack/react-query'

import { useQuery } from '@/lib/connectrpc'
import {
  batchCreateEntitlements,
  getResolvedEntitlementsForAddOn,
  getResolvedEntitlementsForPlanVersion,
  getResolvedEntitlementsForProduct,
  getResolvedEntitlementsForQuote,
  getResolvedEntitlementsForSelection,
  getResolvedEntitlementsForSubscription,
} from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { EntitlementEntitySchema, EntitlementSpecSchema, EntitlementValueSchema } from '@/rpc/api/entitlements/v1/models_pb';

import type { EntitlementEntity } from '@/rpc/api/entitlements/v1/models_pb';

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

/**
 * Persisted entities for which the proto `EntitlementEntity` oneof has a variant.
 * `product` is excluded because the proto has no `productId` field.
 * This is the narrowed type accepted by `useBatchCreateEntitlements`.
 */
export type MutableEntity = Exclude<PersistedEntity, { type: 'product' }>

// Accept a plain-message shape for EntitlementValue (matching PartialMessage convention).
type PartialEntitlementValue = MessageInitShape<typeof EntitlementValueSchema>

// ── Internal helper ───────────────────────────────────────────────────────────

/**
 * Converts a `MutableEntity` to the proto `EntitlementEntity` oneof.
 * `product` is excluded at the type level — no runtime guard needed.
 */
export function toEntitlementEntity(entity: MutableEntity): EntitlementEntity {
  switch (entity.type) {
    case 'add-on':
      return create(
        EntitlementEntitySchema,
        { EntityId: { case: 'addOnId', value: entity.id } }
      );
    case 'plan-version':
      return create(
        EntitlementEntitySchema,
        { EntityId: { case: 'planVersionId', value: entity.id } }
      );
    case 'subscription':
      return create(
        EntitlementEntitySchema,
        { EntityId: { case: 'subscriptionId', value: entity.id } }
      );
    case 'quote':
      return create(
        EntitlementEntitySchema,
        { EntityId: { case: 'quoteId', value: entity.id } }
      );
  }
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

// ── Mutation hook ─────────────────────────────────────────────────────────────

export type BatchCreateSpec = {
  featureId: string
  value: PartialEntitlementValue
}

/**
 * Bulk-creates entitlements on a persisted entity, skipping (feature, entity) conflicts.
 *
 * Accepts `PersistedEntity` for ergonomic call sites (the panel always receives
 * `PersistedEntity`). When `entity.type === 'product'`, the proto has no matching
 * `EntitlementEntity` variant so a noop mutation is returned with `disabled: true`.
 * All other types produce a real mutation that invalidates the resolved-entitlements
 * query for the entity on success.
 */
export const useBatchCreateEntitlements = (entity: PersistedEntity) => {
  const qc = useQueryClient()

  const mutation = useMutation(batchCreateEntitlements, {
    onSuccess: () => {
      switch (entity.type) {
        case 'add-on':
          qc.invalidateQueries({ queryKey: createConnectQueryKey({
            schema: getResolvedEntitlementsForAddOn.parent,
            cardinality: undefined
          }) })
          break
        case 'plan-version':
          qc.invalidateQueries({ queryKey: createConnectQueryKey({
            schema: getResolvedEntitlementsForPlanVersion.parent,
            cardinality: undefined
          }) })
          break
        case 'subscription':
          qc.invalidateQueries({ queryKey: createConnectQueryKey({
            schema: getResolvedEntitlementsForSubscription.parent,
            cardinality: undefined
          }) })
          break
        case 'quote':
          qc.invalidateQueries({ queryKey: createConnectQueryKey({
            schema: getResolvedEntitlementsForQuote.parent,
            cardinality: undefined
          }) })
          break
        default:
          break
      }
    },
  })

  // The proto EntitlementEntity oneof has no productId variant — batch creation
  // is not supported for product surfaces. Return a typed noop so callers can
  // check `disabled` without runtime throws.
  if (entity.type === 'product') {
    return {
      ...mutation,
      mutate: () => {},
      mutateAsync: async () => undefined as never,
      isPending: false as const,
      disabled: true as const,
    }
  }

  const protoEntity = toEntitlementEntity(entity)

  return {
    ...mutation,
    disabled: false as const,
    mutate: (specs: BatchCreateSpec[]) => {
      mutation.mutate({
        entity: protoEntity,
        specs: specs.map(
          s =>
            create(EntitlementSpecSchema, {
              featureId: s.featureId,
              value: create(EntitlementValueSchema, s.value),
            })
        ),
      })
    },
    mutateAsync: async (specs: BatchCreateSpec[]) => {
      return mutation.mutateAsync({
        entity: protoEntity,
        specs: specs.map(
          s =>
            create(EntitlementSpecSchema, {
              featureId: s.featureId,
              value: create(EntitlementValueSchema, s.value),
            })
        ),
      });
    },
  };
}
