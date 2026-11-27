import { skipToken } from '@connectrpc/connect-query'
import { Button } from '@md/ui'
import { Coins, Plus } from 'lucide-react'
import { useNavigate } from 'react-router'

import { SectionEmptyState } from '@/components/empty-state/SectionEmptyState'
import { DetailSection } from '@/components/layouts/DetailLayout'
import { SectionAddButton } from '@/features/customers/cards/ContainedTableSection'
import { useIsDraftVersion, usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { EditPriceComponent } from '@/features/plans/pricecomponents/EditPriceComponent'
import { PriceComponentCard } from '@/features/plans/pricecomponents/PriceComponentCard'
import { ComponentRefsProvider } from '@/features/plans/pricecomponents/componentRefs'
import { useEditedComponents } from '@/features/plans/pricecomponents/utils'
import { useQuery } from '@/lib/connectrpc'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'

export const PriceComponentSection = () => {
  const navigate = useNavigate()

  const planWithVersion = usePlanWithVersion()
  const editedComponents = useEditedComponents()
  const isDraft = useIsDraftVersion()

  const list = useQuery(
    listPriceComponents,
    planWithVersion?.version ? { planVersionId: planWithVersion.version.id } : skipToken
  )?.data
  const priceComponents = list?.components

  return (
    <DetailSection
      title="Pricing"
      description="What subscribers on this plan pay."
      actions={
        isDraft && (
          <SectionAddButton
            label="Add a price component"
            onClick={() => navigate('./add-component')}
          />
        )
      }
    >
      <ComponentRefsProvider list={list}>
        <div className="grid gap-y-4">
          {priceComponents?.map(component =>
            isDraft && editedComponents?.find(id => id === component.id) ? (
              <EditPriceComponent component={component} key={component.id} />
            ) : (
              <PriceComponentCard component={component} key={component.id} />
            )
          )}
          {priceComponents?.length === 0 && (
            <SectionEmptyState
              className="bg-card"
              icon={<Coins />}
              title="No pricing yet"
              action={
                isDraft && (
                  <Button
                    size="sm"
                    variant="secondary"
                    hasIcon
                    onClick={() => navigate('./add-component')}
                  >
                    <Plus size={14} /> Add a price component
                  </Button>
                )
              }
            >
              {isDraft
                ? 'Add a price component to set what subscribers on this plan pay: a flat fee, seats, usage or a one-off charge.'
                : 'This version has no price components. Create a new version to add pricing.'}
            </SectionEmptyState>
          )}
        </div>
      </ComponentRefsProvider>
    </DetailSection>
  )
}
