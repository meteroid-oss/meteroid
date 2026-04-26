import { type PartialMessage } from '@bufbuild/protobuf'

import { type CreateFeatureRequest, type CreateFeatureResponse } from '@/rpc/api/entitlements/v1/entitlements_pb'
import { type EntitlementSpec } from '@/rpc/api/entitlements/v1/models_pb'

import { type PendingEntitlementSpec, pendingSpecToEntitlementSpec } from './types'

// Resolves pending specs into proto EntitlementSpec[].
// For specs with featureName (new): calls createFeature first to get a real featureId.
// Specs with featureId pass through unchanged.
// Partial failure: if createFeature succeeds but the caller's entity creation fails,
// orphaned features remain — harmless, visible in feature list, reusable on retry.
export async function resolveEntitlementSpecs(
  pending: PendingEntitlementSpec[],
  createFeature: (req: PartialMessage<CreateFeatureRequest>) => Promise<CreateFeatureResponse>,
): Promise<EntitlementSpec[]> {
  return Promise.all(
    pending.map(async spec => {
      if (spec.featureId) {
        return pendingSpecToEntitlementSpec(spec as PendingEntitlementSpec & { featureId: string })
      }

      if (!spec.featureName) throw new Error('featureName is required for new feature spec')
      if (spec.featureType === 'metered' && !spec.metricId)
        throw new Error('metricId is required for metered feature spec')

      const res = await createFeature({
        name: spec.featureName,
        featureType:
          spec.featureType === 'boolean'
            ? { Inner: { case: 'boolean', value: {} } }
            : { Inner: { case: 'metered', value: { metricId: spec.metricId } } },
      })

      return pendingSpecToEntitlementSpec({ ...spec, featureId: res.feature!.id })
    })
  )
}
