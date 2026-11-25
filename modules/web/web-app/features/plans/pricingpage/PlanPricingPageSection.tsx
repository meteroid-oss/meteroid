import { InfoIcon } from 'lucide-react'

import { PageSection } from '@/components/layouts/shared/PageSection'
import { usePlanWithVersion } from '@/features/plans/hooks/usePlan'
import { PlanPricingPageForm } from '@/features/plans/pricingpage/PlanPricingPageForm'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { usePermissions } from '@/hooks/usePermissions'

/** The plan's card on its product line's pricing page. Not versioned: any version edits it. */
export const PlanPricingPageSection = () => {
  const { plan, version } = usePlanWithVersion()
  const isExpress = useIsExpressOrganization()
  const { has } = usePermissions()
  if (!plan || !version) return null

  return (
    <PageSection
      header={{
        title: 'Pricing page',
        subtitle:
          'How this plan reads on a public pricing page. Editing it never makes a new version.',
      }}
    >
      {plan.selfServiceRank === undefined && (
        <p className="mb-5 flex items-center gap-2 text-sm text-muted-foreground">
          <InfoIcon size={14} className="shrink-0" />
          Shown once the plan is on the pricing page: add it from the plans list.
        </p>
      )}
      <PlanPricingPageForm
        key={plan.id}
        planId={plan.id}
        planVersionId={plan.activeVersionId ?? version.id}
        readOnly={isExpress || !has('catalog:manage')}
        className="max-w-2xl"
      />
    </PageSection>
  )
}
