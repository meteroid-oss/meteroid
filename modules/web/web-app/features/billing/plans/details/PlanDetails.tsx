import { disableQuery } from '@connectrpc/connect-query'
import { BadgeAlt } from '@ui/components'
import { LinkIcon, PencilIcon } from 'lucide-react'
import { ComponentProps } from 'react'
import { Link } from 'react-router-dom'

import { Property } from '@/components/Property'
import { usePlanOverview } from '@/features/billing/plans/pricecomponents/utils'
import { useQuery } from '@/lib/connectrpc'
import { PlanVersion, PlanStatus, Plan } from '@/rpc/api/plans/v1/models_pb'
import { getLastPublishedPlanVersion } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'

const getStatusBadge = (status: PlanStatus): JSX.Element | null => {
  switch (status) {
    case PlanStatus.ACTIVE:
      return <BadgeAlt color="green">Active</BadgeAlt>
    case PlanStatus.DRAFT:
      return <BadgeAlt color="blue">Draft</BadgeAlt>
    case PlanStatus.ARCHIVED:
      return <BadgeAlt color="gray">Archived</BadgeAlt>
    default:
      return null
  }
}

export const PlanOverview: React.FC<{ plan: Plan; version: PlanVersion }> = ({ plan, version }) => {
  const overview = usePlanOverview()

  const lastPublishedVersion = useQuery(
    getLastPublishedPlanVersion,
    overview?.planId
      ? {
          planId: overview.planId,
        }
      : disableQuery,
    { enabled: !!overview && version.isDraft }
  ).data?.version

  const leftProperties: ComponentProps<typeof Property>[] = [
    version.isDraft
      ? { label: 'Status', value: getStatusBadge(PlanStatus.DRAFT) || 'N/A' }
      : { label: 'Status', value: getStatusBadge(plan.planStatus) || 'N/A' },
    { label: 'External ID', value: plan.externalId },
    lastPublishedVersion && version.isDraft
      ? {
          label: 'Version',
          value: (
            <div className="flex">
              <span className="pr-1">{version.version} (active: </span>
              <Link
                to={`./${lastPublishedVersion.version}`}
                className="flex items-center text-blue-1100 hover:underline"
                target="_blank"
                rel="noreferrer"
              >
                <LinkIcon size={12} strokeWidth={2} className="mr-1" />
                {lastPublishedVersion.version}
              </Link>
              )
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

  if (version.billingConfig?.billingPeriods) {
    rightProperties.push({
      label: 'Billing terms',
      value: (
        <>
          {!overview?.billingPeriods?.length
            ? '_'
            : overview?.billingPeriods?.map(period => (
                <BadgeAlt key={period} color="gray">
                  {period}
                </BadgeAlt>
              ))}
        </>
      ),
    })
  }

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
              {rightProperties.map(property => (
                <Property key={property.label} {...property} />
              ))}
            </div>
          </div>
        </div>
      </div>
      <div className="absolute top-0 right-3 text-slate-900 hover:text-slate-1200 hover:cursor-pointer">
        <PencilIcon size={14} strokeWidth={2} />
      </div>
    </div>
  )
}
