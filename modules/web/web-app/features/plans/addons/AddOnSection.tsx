import { skipToken } from '@connectrpc/connect-query'
import { Button } from '@md/ui'
import { PackagePlus, Plus } from 'lucide-react'
import { useNavigate } from 'react-router'

import { SectionEmptyState } from '@/components/empty-state/SectionEmptyState'
import { DetailSection } from '@/components/layouts/DetailLayout'
import { SectionAddButton } from '@/features/customers/cards/ContainedTableSection'
import { AddOnCard } from '@/features/plans/addons/AddOnCard'
import { useIsDraftVersion, usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { useQuery } from '@/lib/connectrpc'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'

export const AddOnSection = () => {
  const navigate = useNavigate()

  const planWithVersion = usePlanWithVersion()
  const isDraft = useIsDraftVersion()

  const addOns = useQuery(
    listAddOns,
    planWithVersion?.version ? { planVersionId: planWithVersion.version.id } : skipToken
  )?.data?.addOns

  return (
    <DetailSection
      title="Add-ons"
      description="Optional add-ons that can be attached to subscriptions on this plan."
      actions={
        isDraft && (
          <SectionAddButton label="Add an add-on" onClick={() => navigate('./add-addon')} />
        )
      }
    >
      <div className="grid gap-y-4">
        {addOns?.map(addOn => (
          <AddOnCard addOn={addOn} key={addOn.id} />
        ))}
        {addOns?.length === 0 && (
          <SectionEmptyState
            className="bg-card"
            icon={<PackagePlus />}
            title="No add-ons"
            action={
              isDraft && (
                <Button
                  size="sm"
                  variant="secondary"
                  hasIcon
                  onClick={() => navigate('./add-addon')}
                >
                  <Plus size={14} /> Add an add-on
                </Button>
              )
            }
          >
            {isDraft
              ? 'Add-ons are extras subscribers on this plan can buy on top, such as more seats or a support package.'
              : 'Subscribers on this version cannot buy extras. Create a new version to offer add-ons.'}
          </SectionEmptyState>
        )}
      </div>
    </DetailSection>
  )
}
