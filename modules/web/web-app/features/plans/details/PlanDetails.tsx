import { ReactNode } from 'react'
import { Link } from 'react-router'

import { Fact, FactStrip } from '@/components/FactStrip'
import { LocalId } from '@/components/LocalId'
import {
  creditRuleSummary,
  usageDenominations,
  versionChargesUsage,
} from '@/features/plans/creditRule'
import { orderCurrencies, pricedCurrencies } from '@/features/plans/currencies/priceCells'
import { usePlanOverview } from '@/features/plans/hooks/usePlan'
import { formatPrice } from '@/features/plans/pricecomponents/utils'
import { useCreditUnit, useCreditUnits } from '@/features/pricing/CreditUnitPriceFields'
import { effectiveCreditRate } from '@/features/pricing/creditValue'
import { useBasePath } from '@/hooks/useBasePath'
import { entitlementCodes, useEntitlement } from '@/hooks/useEntitlement'
import { useQuery } from '@/lib/connectrpc'
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

/**
 * The version's facts in a strip: when created, its subscriptions, then its settings (currency,
 * payment, prices, credit…). Editing them is in the top bar (see `PlanActions`).
 */
export const PlanFacts: React.FC<{ plan: Plan; version: PlanVersion }> = ({ plan, version }) => {
  const basePath = useBasePath()
  const licensingEnabled = useEntitlement(entitlementCodes.LICENSING)
  const isFree = plan.planType === PlanType.FREE

  const components =
    useQuery(listPriceComponents, { planVersionId: version.id }).data?.components ?? []
  const creditUnitRef = isFree ? undefined : planCreditUnit(components)
  const creditUnit = useCreditUnit(creditUnitRef?.id)
  const units = useCreditUnits()

  const overview = usePlanOverview()
  const subscriptionCount = overview?.subscriptionCount

  const priced = orderCurrencies(
    [version.currency, ...pricedCurrencies(components)],
    version.currency
  )
  const settings: { label: string; value: ReactNode }[] = [
    {
      label: priced.length > 1 ? 'Currencies' : 'Currency',
      value: priced.join(' · '),
    },
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
    settings.push({
      label: 'License',
      value: plan.issuesLicense ? 'Issued per subscription' : 'None',
    })
  }

  return (
    <div className="flex">
      <FactStrip label="Plan settings">
        {!version.isDraft && version.createdAt && (
          <Fact label="Created">{parseAndFormatDate(version.createdAt)}</Fact>
        )}
        {/* A draft isn't live: the plan's subscriptions say nothing about it, so only a published
            version shows them, as it shows when it was created. */}
        {!version.isDraft && subscriptionCount !== undefined && (
          <Fact label="Subscriptions">{subscriptionCount}</Fact>
        )}
        {settings.map(setting => (
          <Fact key={setting.label} label={setting.label}>
            {setting.value}
          </Fact>
        ))}
      </FactStrip>
    </div>
  )
}

/**
 * The line under the plan's name: its ID (shortened, copied whole) and the version shown, with
 * the active version beside a draft.
 */
export const PlanIdentity: React.FC<{
  plan: Plan
  version: PlanVersion
  /** Where the version stands, after it: its rank on the pricing page, the published one… */
  context?: ReactNode
}> = ({ plan, version, context }) => {
  const overview = usePlanOverview()
  const activeVersion = overview?.activeVersion
  return (
    <span className="flex min-w-0 flex-wrap items-center gap-x-2.5 gap-y-1">
      <LocalId
        localId={plan.localId}
        className="max-w-[140px]"
        buttonClassName="h-6 px-2 py-0 text-[11px]"
      />
      <span>
        {version.isDraft ? (
          <>
            Draft of version {version.version}
            {activeVersion && overview && (
              <>
                {' · active '}
                <Link
                  to={`../${overview.localId}/${activeVersion.version}`}
                  className="underline decoration-foreground/30 decoration-dashed underline-offset-[3px] hover:text-foreground"
                >
                  {activeVersion.version}
                </Link>
              </>
            )}
          </>
        ) : (
          `Version ${version.version}`
        )}
      </span>
      {context && <span>{context}</span>}
    </span>
  )
}
