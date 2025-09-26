import { Badge } from '@md/ui'
import { LinkIcon, PencilIcon } from 'lucide-react'
import { ComponentProps } from 'react'
import { Link, useNavigate } from 'react-router-dom'

import { LocalId } from '@/components/LocalId'
import { Property } from '@/components/Property'
import { useIsDraftVersion, usePlanOverview } from '@/features/plans/hooks/usePlan'
import { Plan, PlanStatus, PlanType, PlanVersion } from '@/rpc/api/plans/v1/models_pb'

const getStatusBadge = (status: PlanStatus): JSX.Element | null => {
  switch (status) {
    case PlanStatus.ACTIVE:
      return <Badge variant="success">Active</Badge>
    case PlanStatus.DRAFT:
      return <Badge variant="primary">Draft</Badge>
    case PlanStatus.ARCHIVED:
      return <Badge variant="secondary">Archived</Badge>
    default:
      return null
  }
}

export const PlanOverview: React.FC<{ plan: Plan; version: PlanVersion }> = ({ plan, version }) => {
  const overview = usePlanOverview()
  const isDraft = useIsDraftVersion()
  const navigate = useNavigate()

  const leftProperties: ComponentProps<typeof Property>[] = [
    version.isDraft
      ? { label: 'Status', value: getStatusBadge(PlanStatus.DRAFT) || 'N/A' }
      : { label: 'Status', value: getStatusBadge(plan.planStatus) || 'N/A' },
    {
      label: 'Plan handle',
      value: <LocalId localId={plan.localId} className="max-w-28" />,
    },
    overview && isDraft
      ? {
          label: 'Version',
          value: (
            <div className="flex gap-2">
              <span className="pr-1">{version.version}</span>
              {overview.activeVersion && (
                <>
                  <span className="pr-1">(active: </span>
                  <Link
                    to={`../${overview.localId}/${overview.activeVersion.version}`}
                    className="flex items-center text-blue-1100 hover:underline"
                  >
                    <LinkIcon size={12} strokeWidth={2} className="mr-1" />
                    {overview.activeVersion.version}
                  </Link>
                  )
                </>
              )}
            </div>
          ),
        }
      : { label: 'Version', value: version.version },
    { label: 'Description', value: plan.description?.length ? plan.description : '_' },
  ]

  const rightProperties: ComponentProps<typeof Property>[] = [
    { label: 'Currency', value: version.currency },
    {
      label: 'Net terms',
      value: version.billingConfig?.netTerms
        ? `Net ${version.billingConfig.netTerms}`
        : 'Due on issue',
    },
  ]

  return (
    <div className="flex pb-6 mb-6 relative">
      <div className="flex flex-col gap-y-4 w-full flex-none">
        <div className="flex-col gap-x-4">
          <div className="flex max-lg:flex-col lg:flex-row gap-x-36 gap-y-2">
            <div className="flex flex-col gap-y-2">
              {leftProperties.map(property => (
                <Property key={property.label} {...property} />
              ))}
            </div>
            <div className="flex flex-col gap-y-2">
              {plan.planType !== PlanType.FREE &&
                rightProperties.map(property => <Property key={property.label} {...property} />)}
            </div>
          </div>
        </div>
      </div>
      <div className="absolute top-0 right-3 text-muted-foreground hover:text-foreground hover:cursor-pointer">
        <PencilIcon size={14} strokeWidth={2} onClick={() => navigate('edit-overview')} />
      </div>
    </div>
  )
}
