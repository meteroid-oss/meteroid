import { disableQuery } from '@connectrpc/connect-query'
import { Button } from '@md/ui'
import { useNavigate } from 'react-router-dom'

import { PageSection } from '@/components/layouts/shared/PageSection'
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
    planWithVersion?.version
      ? { planVersionId: planWithVersion.version.id }
      : disableQuery
  )?.data?.addOns

  return (
    <PageSection
      header={{
        title: 'Add-ons',
        subtitle: 'Optional add-ons that can be attached to subscriptions on this plan',
        actions: isDraft ? (
          <Button
            variant="outline"
            onClick={() => navigate('./add-addon')}
            className="py-1.5"
          >
            + Add an add-on
          </Button>
        ) : null,
      }}
    >
      <div className="grid gap-y-4">
        {addOns?.map(addOn => (
          <AddOnCard addOn={addOn} key={addOn.id} />
        ))}
        {addOns?.length === 0 && (
          <span className="text-muted-foreground text-sm">No add-ons</span>
        )}
      </div>
    </PageSection>
  )
}
