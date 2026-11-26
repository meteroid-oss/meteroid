import { skipToken } from '@connectrpc/connect-query'
import { Sheet, SheetContent, SheetHeader, SheetTitle, Skeleton } from '@md/ui'
import { Outlet, useNavigate, useParams } from 'react-router'

import { PanelEmptyState } from '@/components/empty-state/PanelEmptyState'
import { FeatureCreateSheet } from '@/features/entitlements/features/FeatureCreateSheet'
import { FeatureDetailSheet } from '@/features/entitlements/features/FeatureDetailSheet'
import { FeaturesPage } from '@/features/entitlements/features/FeaturesPage'
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
  const navigate = useNavigate()
  const featureQuery = useQuery(getFeature, featureId ? { id: featureId } : skipToken)
  const feature = featureQuery.data?.feature

  if (feature) return <FeatureCreateSheet key={feature.id} feature={feature} />

  return (
    <Sheet open onOpenChange={() => navigate('..')}>
      <SheetContent size="xlarge">
        <SheetHeader>
          <SheetTitle>Edit feature</SheetTitle>
        </SheetHeader>
        {featureQuery.isLoading ? (
          <div className="flex flex-col gap-4 py-4">
            <Skeleton className="h-9 w-full max-w-xl" />
            <Skeleton className="h-20 w-full max-w-xl" />
            <Skeleton className="h-9 w-full max-w-xl" />
          </div>
        ) : (
          <PanelEmptyState title="Feature not found" />
        )}
      </SheetContent>
    </Sheet>
  )
}
