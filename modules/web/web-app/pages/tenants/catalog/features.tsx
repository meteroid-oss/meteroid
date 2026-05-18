import { disableQuery } from '@connectrpc/connect-query'
import { Skeleton } from '@md/ui'
import { useParams , Outlet } from 'react-router-dom'

import { FeatureCreateSheet } from '@/features/entitlements/features/FeatureCreateSheet'
import { FeatureDetailSheet } from '@/features/entitlements/features/FeatureDetailSheet'
import { FeaturesPage } from '@/features/entitlements/features/FeaturesPage'
import { featureKindFromProto } from '@/features/entitlements/utils'
import { useQuery } from '@/lib/connectrpc'
import { getFeature } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'

export const Features = () => (
  <>
    <FeaturesPage />
    <Outlet />
  </>
)

export const FeatureCreate = () => <FeatureCreateSheet />

export const FeatureDetail = () => <FeatureDetailSheet />

export const FeatureEdit = () => {
  const { featureId } = useParams<{ featureId: string }>()
  const featureQuery = useQuery(getFeature, featureId ? { id: featureId } : disableQuery)
  const feature = featureQuery.data?.feature

  if (featureQuery.isLoading || !feature) {
    return <Skeleton className="h-10 w-48" />
  }

  return (
    <FeatureCreateSheet
      featureId={featureId}
      initialName={feature.name}
      initialDescription={feature.description}
      initialProductId={feature.product?.id}
      initialKind={featureKindFromProto(feature.featureType)}
    />
  )
}
