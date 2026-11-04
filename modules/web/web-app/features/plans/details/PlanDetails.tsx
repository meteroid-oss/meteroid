import { Button, Tooltip, TooltipContent, TooltipTrigger } from '@md/ui'
import { CopyIcon, PencilIcon } from 'lucide-react'
import { ReactNode } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import {
  creditRuleSummary,
  usageDenominations,
  versionChargesUsage,
} from '@/features/plans/creditRule'
import { usePlanOverview } from '@/features/plans/hooks/usePlan'
import { formatPrice } from '@/features/plans/pricecomponents/utils'
import { useCreditUnit, useCreditUnits } from '@/features/pricing/CreditUnitPriceFields'
import { effectiveCreditRate } from '@/features/pricing/creditValue'
import { useBasePath } from '@/hooks/useBasePath'
import { entitlementCodes, useEntitlement } from '@/hooks/useEntitlement'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { useQuery } from '@/lib/connectrpc'
import { copyToClipboard } from '@/lib/helpers'
import { Plan, PlanType, PlanVersion } from '@/rpc/api/plans/v1/models_pb'
import { listPriceComponents } from '@/rpc/api/pricecomponents/v1/pricecomponents-PriceComponentsService_connectquery'
import { parseAndFormatDate } from '@/utils/date'

import type { PriceComponent } from '@/rpc/api/pricecomponents/v1/models_pb'

// Subscriptions inherit the plan's net terms; whether they charge automatically is set on each.
export const planPaymentLabel = (netTerms: number): string =>
  netTerms > 0 ? `Net ${netTerms} days` : 'Due on receipt'

/** The credit unit the version's prices use, with its overage rate when a usage price sets one. */
const planCreditUnit = (components: PriceComponent[]) => {
  const prices = components.flatMap(component => component.prices)
  const packUnitIds = prices.flatMap(price =>
    price.pricing.case === 'creditPricing' && price.pricing.value.creditUnitId
      ? [price.pricing.value.creditUnitId]
      : []
  )
  const id = prices.find(price => price.creditUnit)?.creditUnit?.creditUnitId ?? packUnitIds[0]
  if (!id) return undefined
  const rate = prices.find(price => price.creditUnit?.creditUnitId === id)?.creditUnit?.rate
  return { id, rate }
}

/** " · 1 credit = $0.01", never a zero value: " · No money value" without one. */
const creditValueLabel = (rate: string | undefined, currency: string) =>
  rate ? ` · 1 credit = ${formatPrice(currency)(rate)}` : ' · No money value'

export const PlanOverview: React.FC<{ plan: Plan; version: PlanVersion }> = ({ plan, version }) => {
  const navigate = useNavigate()
  const basePath = useBasePath()
  const isExpress = useIsExpressOrganization()
  const licensingEnabled = useEntitlement(entitlementCodes.LICENSING)
  const isFree = plan.planType === PlanType.FREE

  const components =
    useQuery(listPriceComponents, { planVersionId: version.id }).data?.components ?? []
  const creditUnitRef = isFree ? undefined : planCreditUnit(components)
  const creditUnit = useCreditUnit(creditUnitRef?.id)
  const units = useCreditUnits()

  const settings: { label: string; value: ReactNode }[] = [
    { label: 'Currency', value: version.currency },
  ]
  if (!isFree) {
    if (creditUnit) {
      settings.push({
        label: 'Credit unit',
        value: (
          <>
            <Link to={`${basePath}/settings/credit-units`} className="text-brand hover:underline">
              {creditUnit.pluralName}
            </Link>
            {creditValueLabel(
              effectiveCreditRate(creditUnitRef?.rate, creditUnit, version.currency),
              version.currency
            )}
          </>
        ),
      })
    }
    settings.push(
      {
        label: 'Payment',
        value: planPaymentLabel(version.netTerms),
      },
      { label: 'Prices', value: version.taxInclusive ? 'Tax included' : 'Tax excluded' }
    )
    if (versionChargesUsage(components)) {
      settings.push({
        label: 'When credit runs out',
        value: creditRuleSummary(
          version.creditExhaustedRule,
          components,
          usageDenominations(components, version.currency, units)
        ),
      })
    }
  }
  if (licensingEnabled || plan.issuesLicense) {
    settings.push({ label: 'License', value: plan.issuesLicense ? 'Issued per subscription' : 'None' })
  }

  return (
    <div className="flex flex-col gap-4 pb-6">
      {plan.description?.trim() && (
        <p className="text-sm text-muted-foreground max-w-3xl">{plan.description}</p>
      )}
      <div className="flex items-start gap-2">
        <section
          aria-label="Plan settings"
          className="flex flex-1 flex-wrap rounded-lg border border-border bg-card divide-x divide-border"
        >
          {settings.map(setting => (
            <div key={setting.label} className="flex min-w-40 flex-1 flex-col gap-1 px-4 py-3">
              <span className="text-xs text-muted-foreground">{setting.label}</span>
              <span className="text-sm text-foreground">{setting.value}</span>
            </div>
          ))}
        </section>
        {!isExpress && (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                aria-label="Edit plan details"
                className="text-muted-foreground hover:text-foreground"
                onClick={() => navigate('edit-overview')}
              >
                <PencilIcon size={14} strokeWidth={2} />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Edit plan details</TooltipContent>
          </Tooltip>
        )}
      </div>
    </div>
  )
}

export const PlanVersionFooter: React.FC<{ plan: Plan; version: PlanVersion }> = ({
  plan,
  version,
}) => {
  const overview = usePlanOverview()
  const subscriptionCount = overview?.subscriptionCount
  const activeVersion = overview?.activeVersion

  const parts: ReactNode[] = version.isDraft
    ? [`Draft of version ${version.version}`]
    : [
        `Version ${version.version}`,
        ...(version.createdAt ? [`created ${parseAndFormatDate(version.createdAt)}`] : []),
      ]
  if (version.isDraft && activeVersion && overview) {
    parts.push(
      <>
        active version{' '}
        <Link
          to={`../${overview.localId}/${activeVersion.version}`}
          className="text-brand hover:underline"
        >
          {activeVersion.version}
        </Link>
      </>
    )
  }
  if (subscriptionCount !== undefined) {
    parts.push(`${subscriptionCount} ${subscriptionCount === 1 ? 'subscription' : 'subscriptions'}`)
  }

  return (
    <footer className="flex flex-wrap items-center justify-between gap-2 border-t border-border pt-4 text-xs text-muted-foreground">
      <span>
        {parts.map((part, index) => (
          <span key={index}>
            {index > 0 && ' · '}
            {part}
          </span>
        ))}
      </span>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            hasIcon
            aria-label="Copy plan ID"
            className="mt-0 h-7 px-2 text-xs font-normal text-muted-foreground hover:text-foreground"
            onClick={() =>
              copyToClipboard(plan.localId, () => toast.success('Plan ID copied to clipboard'))
            }
          >
            <span className="max-w-40 truncate">ID {plan.localId}</span>
            <CopyIcon size={10} />
          </Button>
        </TooltipTrigger>
        <TooltipContent>Copy plan ID</TooltipContent>
      </Tooltip>
    </footer>
  )
}
