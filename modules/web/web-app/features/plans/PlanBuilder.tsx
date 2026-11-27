import { skipToken } from '@connectrpc/connect-query'
import { InfoIcon } from '@md/icons'
import { Button, Tabs, TabsContent, TabsList, TabsTrigger, cn } from '@md/ui'
import { PaginationState } from '@tanstack/react-table'
import { ScopeProvider } from 'jotai-scope'
import { AlertTriangleIcon, ArrowRight, Layers, Plus, Repeat } from 'lucide-react'
import { ReactNode, useRef, useState } from 'react'
import { Link, useNavigate, useParams } from 'react-router'

import { InlineLoading } from '@/components/Loading'
import { SectionEmptyState } from '@/components/empty-state/SectionEmptyState'
import {
  DetailIcon,
  DetailMain,
  DetailPageFrame,
  DetailSection,
  DetailSections,
  DetailTitle,
} from '@/components/layouts/DetailLayout'
import { SimpleTable } from '@/components/table/SimpleTable'
import { Slot, hasSlot } from '@/edition'
import {
  ContainedTableSection,
  SectionAddButton,
} from '@/features/customers/cards/ContainedTableSection'
import {
  ResolvedEntitlementsPanel,
  ResolvedEntitlementsPanelHandle,
} from '@/features/entitlements/resolved/ResolvedEntitlementsPanel'
import { ListPlanVersionTab } from '@/features/plans/ListPlanVersion'
import { PlanActions } from '@/features/plans/PlanActions'
import { PlanStatusBadge } from '@/features/plans/PlanStatusBadge'
import { AddOnSection } from '@/features/plans/addons/AddOnSection'
import { useCreditPaidLabels } from '@/features/plans/creditPaidMetrics'
import { PlanFacts, PlanIdentity } from '@/features/plans/details/PlanDetails'
import {
  useIsDraftVersion,
  usePlanOverview,
  usePlanWithVersion,
} from '@/features/plans/hooks/usePlan'
import { PlanMinimumCommitmentDisplay } from '@/features/plans/minimum/PlanMinimumCommitment'
import { PriceComponentSection } from '@/features/plans/pricecomponents/PriceComponentSection'
import { editedComponentsAtom } from '@/features/plans/pricecomponents/utils'
import { PlanTrial } from '@/features/plans/trial/PlanTrial'
import { CreditRateOptionalContext } from '@/features/pricing/CreditUnitPriceFields'
import { PurchaseLinksSection } from '@/features/purchase-links/PurchaseLinksSection'
import { SubscriptionsTable } from '@/features/subscriptions'
import { useBasePath } from '@/hooks/useBasePath'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { useQueryState } from '@/hooks/useQueryState'
import { useScrollFade } from '@/hooks/useScrollFade'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { CreditExhaustedRule_Kind, PlanStatus, PlanType } from '@/rpc/api/plans/v1/models_pb'
import { listSubscriptions } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

interface Props {
  children?: ReactNode
}

export const PlanBuilder: React.FC<Props> = ({ children }) => {
  const basePath = useBasePath()
  const { planVersion } = useParams<{ planVersion?: string }>()

  const isDraft = useIsDraftVersion()
  const overview = usePlanOverview()
  const planData = usePlanWithVersion()
  const { version } = planData

  // Only show the draft alert when:
  // 1. Not viewing the draft itself (!isDraft)
  // 2. There is a draft version (hasDraftVersion)
  // 3. We're on the default view (no planVersion param)
  //    This excludes specific numbered versions like /plans/123/2
  const showDraftAlert = !isDraft && overview?.hasDraftVersion && !planVersion

  // A version that has not chosen its rule pauses by default once it prices usage in credits.
  const rateOptional =
    !version?.creditExhaustedRule ||
    version.creditExhaustedRule.kind === CreditExhaustedRule_Kind.PAUSE

  // The open tab, kept in the URL as on the customer page.
  const [requestedTab, setTab] = useQueryState('tab', 'overview', { replace: true })
  const tab = PLAN_TABS.some(t => t.id === requestedTab) ? requestedTab : 'overview'
  const tabFade = useScrollFade<HTMLDivElement>()

  return (
    <ScopeProvider atoms={[editedComponentsAtom]}>
      <CreditRateOptionalContext.Provider value={rateOptional}>
        <DetailPageFrame
          section={{ label: 'Plans', to: `${basePath}/plans` }}
          title={
            <span className="flex items-center gap-2">
              <span className="truncate">{overview?.name}</span>
              {version?.isDraft ? (
                <PlanStatusBadge status={PlanStatus.DRAFT} />
              ) : (
                overview && <PlanStatusBadge status={overview.planStatus} />
              )}
            </span>
          }
          actions={<PlanActions />}
        >
          {/* No sidebar: the version's settings sit in a strip under the title. */}
          <DetailMain sidebar={false}>
            {/* A draft with unpublished changes exists: said first, above the plan, with the way in. */}
            {showDraftAlert && (
              <div className="mb-6 flex items-center gap-3 rounded-lg border border-brand/25 bg-brand/[0.08] py-2 pl-4 pr-2">
                <InfoIcon size={16} className="shrink-0 text-brand" />
                <span className="min-w-0 flex-1 text-[13px] text-foreground">
                  A draft version of this plan has unpublished changes.
                </span>
                <Button size="sm" variant="secondary" hasIcon asChild>
                  <Link to={`../${overview.localId}/draft`}>
                    Open draft <ArrowRight size={14} className="text-muted-foreground" />
                  </Link>
                </Button>
              </div>
            )}
            {overview && (
              <DetailTitle
                media={
                  <DetailIcon>
                    <Layers />
                  </DetailIcon>
                }
                title={overview.name}
                subtitle={
                  planData.plan &&
                  version && <PlanIdentity plan={planData.plan} version={version} />
                }
              />
            )}
            {planData.plan?.description?.trim() && (
              <p className="-mt-2 mb-2 max-w-3xl text-[13px] leading-relaxed text-muted-foreground">
                {planData.plan.description}
              </p>
            )}
            {planData.plan && version && (
              <div className="mb-4">
                <PlanFacts plan={planData.plan} version={version} />
              </div>
            )}

            {isDraft && version && !version.usesProductPricing && (
              <div className="relative">
                <div className="opacity-20 pointer-events-none select-none" aria-hidden>
                  <PlanBody />
                </div>
                <div className="absolute inset-0 flex items-start justify-center pt-16">
                  <OutdatedDraftAlert />
                </div>
              </div>
            )}
            {isDraft && (!version || version.usesProductPricing) && <PlanBody />}
            {!isDraft && (
              <>
                <Tabs value={tab} onValueChange={setTab} className="w-full">
                  <TabsList
                    ref={tabFade.ref}
                    variant="underline"
                    className={cn(
                      'w-full overflow-x-auto',
                      tabFade.fadeLeft && 'scroll-fade-x-left',
                      tabFade.fadeRight && 'scroll-fade-x-right'
                    )}
                  >
                    {PLAN_TABS.map(t => (
                      <TabsTrigger key={t.id} value={t.id}>
                        {t.label}
                      </TabsTrigger>
                    ))}
                  </TabsList>
                  <TabsContent value="overview" className="mt-6">
                    <PlanBody />
                  </TabsContent>
                  <TabsContent value="subscriptions" className="mt-6">
                    <SubscriptionsTab />
                  </TabsContent>
                  <TabsContent value="alerts" className="mt-6">
                    <p className="text-sm text-muted-foreground">Alerts are not implemented yet</p>
                  </TabsContent>
                  <TabsContent value="versions" className="mt-6">
                    <ContainedTableSection title="Versions">
                      <ListPlanVersionTab />
                    </ContainedTableSection>
                  </TabsContent>
                </Tabs>
              </>
            )}
          </DetailMain>
        </DetailPageFrame>
        {children}
      </CreditRateOptionalContext.Provider>
    </ScopeProvider>
  )
}

// The tabs of a published version, as before: its details, its subscriptions, alerts, history.
const PLAN_TABS = [
  { id: 'overview', label: 'Details' },
  { id: 'subscriptions', label: 'Subscriptions' },
  { id: 'alerts', label: 'Alerts' },
  { id: 'versions', label: 'History' },
]

const OutdatedDraftAlert = () => {
  return (
    <div className="flex flex-col items-center justify-center py-16 gap-4">
      <div className="flex items-center gap-2 text-warning">
        <AlertTriangleIcon size={20} />
        <span className="text-sm font-medium">Outdated draft</span>
      </div>
      <p className="text-sm text-muted-foreground text-center max-w-md">
        This draft was created before a pricing model update and is no longer editable. Please
        discard it and create a new version from the published plan.
      </p>
    </div>
  )
}

const SubscriptionsTab = () => {
  const overview = usePlanOverview()
  const planData = usePlanWithVersion()
  const isExpress = useIsExpressOrganization()

  const navigate = useNavigate()
  const basePath = useBasePath()

  const [pagination, setPagination] = useState<PaginationState>({
    pageIndex: 0,
    pageSize: 15,
  })

  const subscriptionsQuery = useQuery(
    listSubscriptions,
    overview
      ? {
          planId: overview.id,
          pagination: {
            perPage: pagination.pageSize,
            page: pagination.pageIndex,
          },
          status: [],
        }
      : skipToken
  )

  const data = subscriptionsQuery.data?.subscriptions ?? []
  const count = Number(subscriptionsQuery.data?.paginationMeta?.totalItems ?? 0)
  const isLoading = subscriptionsQuery.isLoading

  return (
    <ContainedTableSection
      title="Subscriptions"
      actionLabel="New subscription"
      onAction={
        !isExpress
          ? () =>
              navigate(`${basePath}/subscriptions/create?planVersionId=${planData?.version?.id}`)
          : undefined
      }
    >
      {!isLoading && count === 0 ? (
        <SectionEmptyState
          framed={false}
          icon={<Repeat />}
          title="No subscriptions on this plan"
          action={
            !isExpress && (
              <Button
                size="sm"
                variant="secondary"
                hasIcon
                onClick={() =>
                  navigate(
                    `${basePath}/subscriptions/create?planVersionId=${planData?.version?.id}`
                  )
                }
              >
                <Plus size={14} /> New subscription
              </Button>
            )
          }
        >
          Customers subscribed to any version of this plan show here.
        </SectionEmptyState>
      ) : (
        <SubscriptionsTable
          data={data}
          totalCount={count}
          pagination={pagination}
          setPagination={setPagination}
          isLoading={isLoading}
          hidePlan
        />
      )}
    </ContainedTableSection>
  )
}

const EntitlementsSectionForPlanVersion = ({ planVersionId }: { planVersionId: string }) => {
  const panelRef = useRef<ResolvedEntitlementsPanelHandle>(null)
  const creditPaidLabels = useCreditPaidLabels(planVersionId)
  return (
    <DetailSection
      title="Entitlements"
      description="Features and limits applied to this plan version."
      actions={
        <SectionAddButton label="Add entitlement" onClick={() => panelRef.current?.openAdd()} />
      }
    >
      <ResolvedEntitlementsPanel
        ref={panelRef}
        entity={{ type: 'plan-version', id: planVersionId }}
        canPin={true}
        hideAddButton
        hidePinAll
        meteredLabels={creditPaidLabels}
      />
    </DetailSection>
  )
}

const PlanBody = () => {
  const planData = usePlanWithVersion()

  if (planData.isLoading) {
    return <InlineLoading />
  }

  if (!planData?.plan || !planData.version) {
    return <>Failed to load plan</>
  }

  const plan = planData.plan
  const current = planData.version

  return (
    // Sections 32px apart with a divider halfway, as on the customer page.
    <DetailSections>
      {plan.planType !== PlanType.FREE && <PriceComponentSection />}

      {plan.planType !== PlanType.FREE &&
        current &&
        (hasSlot('planDetail.minimumCommitment') || current.minimumCommitment) && (
          <DetailSection
            title="Contract terms"
            description="Commercial terms applied to subscriptions on this plan. A minimum guarantees an invoice amount per billing period."
          >
            {hasSlot('planDetail.minimumCommitment') ? (
              <Slot name="planDetail.minimumCommitment" planVersion={current} />
            ) : (
              <PlanMinimumCommitmentDisplay planVersion={current} />
            )}
          </DetailSection>
        )}

      <DetailSection
        title="Trial"
        description={
          plan.planType === PlanType.FREE
            ? 'Give users temporary access to premium features before they continue on this free plan.'
            : 'Define a trial period for new subscribers.'
        }
      >
        <PlanTrial
          config={current?.trialConfig}
          currentPlanId={plan.id}
          currentPlanVersionId={current.id}
          planType={plan.planType}
        />
      </DetailSection>

      {plan.planType !== PlanType.FREE && (
        <>
          <DetailSection hidden title="Schedules" description="Define the phases of your plan.">
            <div className="space-x-4 ">
              <SimpleTable columns={[]} data={[]} emptyMessage="No schedule configured" />
            </div>
          </DetailSection>
          <DetailSection
            hidden
            title="Price points"
            description="Define alternative prices and currencies for this plans, for specific countries or audiences."
          >
            <Tabs defaultValue="localizations" className="w-full">
              <TabsList className="w-full justify-start">
                <TabsTrigger value="localizations">Localizations</TabsTrigger>
                <TabsTrigger value="audiences">Audiences</TabsTrigger>
                <TabsTrigger value="experimentations">Experimentations</TabsTrigger>
              </TabsList>
              <TabsContent value="localizations" className="pt-4">
                <SimpleTable
                  headTrClasses="!hidden"
                  columns={[]}
                  data={[]}
                  emptyMessage="No price point"
                />
              </TabsContent>
              <TabsContent value="audiences">Not implemented. Upvote TODO</TabsContent>
              <TabsContent value="experimentations">Not implemented. Upvote TODO</TabsContent>
            </Tabs>
          </DetailSection>
        </>
      )}

      <AddOnSection />

      <PurchaseLinksSection />

      {env.entitlementsEnabled && <EntitlementsSectionForPlanVersion planVersionId={current.id} />}

      <Slot name="planDetail.sections" planVersionId={current.id} />
    </DetailSections>
  )
}
