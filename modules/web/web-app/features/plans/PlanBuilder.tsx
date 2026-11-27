import { skipToken } from '@connectrpc/connect-query'
import { InfoIcon } from '@md/icons'
import { Button, Tabs, TabsContent, TabsList, TabsTrigger, cn } from '@md/ui'
import { PaginationState } from '@tanstack/react-table'
import { ScopeProvider } from 'jotai-scope'
import {
  AlertTriangleIcon,
  ArrowRight,
  InfoIcon as InfoLineIcon,
  LayoutTemplateIcon,
  Layers,
  LinkIcon,
  Plus,
  PuzzleIcon,
  Repeat,
  TicketIcon,
  TimerIcon,
  TrendingUpIcon,
  UserCheckIcon,
} from 'lucide-react'
import { ReactNode, useState } from 'react'
import { Link, useLocation, useNavigate, useParams } from 'react-router'

import { InlineLoading } from '@/components/Loading'
import { SectionEmptyState } from '@/components/empty-state/SectionEmptyState'
import {
  DetailIcon,
  DetailMain,
  DetailPageFrame,
  DetailSections,
  DetailTitle,
} from '@/components/layouts/DetailLayout'
import { Slot, hasSlot, slotItems } from '@/edition'
import { ContainedTableSection } from '@/features/customers/cards/ContainedTableSection'
import { eligibilitySummary, isOpen } from '@/features/eligibility/eligibility'
import { ListPlanVersionTab } from '@/features/plans/ListPlanVersion'
import { AddRow, MoreForPlan } from '@/features/plans/MoreForPlan'
import { PlanActions } from '@/features/plans/PlanActions'
import { PlanEntitlementsSection } from '@/features/plans/PlanEntitlementsSection'
import { PlanStatusBadge } from '@/features/plans/PlanStatusBadge'
import { AddOnSection } from '@/features/plans/addons/AddOnSection'
import { LockedPrices } from '@/features/plans/canvas/LockedPrices'
import { SellingGapsBanner } from '@/features/plans/canvas/SellingGapsBanner'
import { PlanFacts, PlanIdentity } from '@/features/plans/details/PlanDetails'
import { PlanEligibility } from '@/features/plans/eligibility/PlanEligibility'
import {
  useIsDraftVersion,
  usePlanOverview,
  usePlanWithVersion,
} from '@/features/plans/hooks/usePlan'
import { PlanMinimumCommitmentDisplay } from '@/features/plans/minimum/PlanMinimumCommitment'
import { PriceComponentSection } from '@/features/plans/pricecomponents/PriceComponentSection'
import { editedComponentsAtom, priceCellRequestAtom } from '@/features/plans/pricecomponents/utils'
import { PlanPricingPageForm } from '@/features/plans/pricingpage/PlanPricingPageForm'
import { PlanTrial } from '@/features/plans/trial/PlanTrial'
import { usePlanCoupons } from '@/features/plans/usePlanCoupons'
import { CreditRateOptionalContext } from '@/features/pricing/CreditUnitPriceFields'
import {
  couponDiscountLabel,
  couponDurationLabel,
} from '@/features/productCatalog/coupons/discountLabel'
import { PurchaseLinksSection } from '@/features/purchase-links/PurchaseLinksSection'
import { getCountryName } from '@/features/settings/utils'
import { SubscriptionsTable } from '@/features/subscriptions'
import { useBasePath } from '@/hooks/useBasePath'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { usePermissions } from '@/hooks/usePermissions'
import { useQueryState } from '@/hooks/useQueryState'
import { useScrollFade } from '@/hooks/useScrollFade'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { formatCurrency } from '@/lib/utils/numbers'
import { listAddOns } from '@/rpc/api/addons/v1/addons-AddOnsService_connectquery'
import { CreditExhaustedRule_Kind, PlanStatus, PlanType } from '@/rpc/api/plans/v1/models_pb'
import { listPlans } from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import { getPlanPricingPage } from '@/rpc/api/pricingpage/v1/pricingpage-PricingPageService_connectquery'
import { listPurchaseLinks } from '@/rpc/api/purchaselinks/v1/purchaselinks-PurchaseLinksService_connectquery'
import { listSubscriptions } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'

import type { PlanExtra } from '@/features/plans/MoreForPlan'
import type { Coupon } from '@/rpc/api/coupons/v1/models_pb'

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
  const context = usePlanContext()

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
    <ScopeProvider atoms={[editedComponentsAtom, priceCellRequestAtom]}>
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
          actions={<PlanActions onHistory={isDraft ? undefined : () => setTab('versions')} />}
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
                  version && (
                    <PlanIdentity plan={planData.plan} version={version} context={context} />
                  )
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

/** Where the version stands, beside its ID: its rank on the pricing page, or the published one. */
const usePlanContext = () => {
  const overview = usePlanOverview()
  const { plan, version } = usePlanWithVersion()
  const isDraft = useIsDraftVersion()
  const family = overview?.productFamilyLocalId
  const rank = overview?.selfServiceRank
  const ranked =
    useQuery(
      listPlans,
      !isDraft && rank !== undefined && family
        ? { productFamilyLocalId: family, pagination: { perPage: 100, page: 0 } }
        : skipToken
    ).data?.plans.filter(p => p.selfServiceRank !== undefined).length ?? 0
  const page = useQuery(
    getPlanPricingPage,
    !isDraft && rank !== undefined && plan ? { planId: plan.id } : skipToken
  ).data?.page

  const live = overview?.activeVersion
  const subscriptions = overview?.subscriptionCount ?? 0
  const isLive = !!version && live?.version === version.version
  if (!version) return undefined
  if (isDraft) {
    if (!live) return 'Never published'
    return subscriptions > 0
      ? `${subscriptions} ${subscriptions === 1 ? 'subscription stays' : 'subscriptions stay'} on v${live.version} until migrated`
      : undefined
  }
  if (rank !== undefined) {
    return [
      ranked > 0 ? `Rank ${rank} of ${Math.max(ranked, rank)}` : `Rank ${rank}`,
      page?.popular && 'Popular',
    ]
      .filter(Boolean)
      .join(' · ')
  }
  return !isLive && live ? `v${live.version} is the published one` : undefined
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

const PlanBody = () => {
  const planData = usePlanWithVersion()
  const isDraft = useIsDraftVersion()
  const plan = planData.plan
  const current = planData.version
  const extras = usePlanExtras()

  if (planData.isLoading) {
    return <InlineLoading />
  }

  if (!plan || !current) {
    return <>Failed to load plan</>
  }

  const priced = plan.planType !== PlanType.FREE
  return (
    <div className="flex flex-col gap-4">
      {priced && <SellingGapsBanner />}
      {/* Sections 32px apart with a divider halfway, as on the customer page. */}
      <DetailSections>
        {priced && !isDraft && <LockedPrices />}
        {priced && isDraft && (
          <div id="pricing">
            <PriceComponentSection />
          </div>
        )}

        {env.entitlementsEnabled && (
          <PlanEntitlementsSection plan={plan} planVersionId={current.id} isDraft={isDraft} />
        )}

        <MoreForPlan extras={extras} />

        <Slot name="planDetail.sections" planId={plan.id} planVersionId={current.id} />
      </DetailSections>
    </div>
  )
}

const COUPONS_LISTED = 20

/** "Spring sale · 20% off · for 3 months and 41 more"; single-use codes are only counted. */
const couponsSummary = (featured: Coupon | undefined, total: number) => {
  if (!featured) return `${total} apply`
  const named = `${featured.description || featured.code} · ${couponTermsLabel(featured)}`
  return total > 1 ? `${named} and ${total - 1} more` : named
}

/** The live coupons that apply to this plan, the most redeemed first, each linking to its page. */
const PlanCoupons = ({
  coupons,
  total,
  newCouponPath,
}: {
  coupons: Coupon[]
  total: number
  newCouponPath: string
}) => {
  const basePath = useBasePath()
  const navigate = useNavigate()
  return (
    <div className="space-y-3">
      {coupons.length === 0 ? (
        <p className="text-sm text-muted-foreground">No live coupon applies to this plan.</p>
      ) : (
        <ul className="divide-y divide-border rounded-md border border-border">
          {coupons.map(coupon => (
            <li key={coupon.id} className="flex items-center justify-between gap-4 px-3 py-2.5">
              <span className="min-w-0">
                <Link
                  to={`${basePath}/coupons/${coupon.localId}`}
                  className="block truncate text-sm font-medium hover:underline"
                >
                  {coupon.description || coupon.code}
                </Link>
                {coupon.description && (
                  <span className="block truncate font-mono text-xs text-muted-foreground">
                    {coupon.code}
                  </span>
                )}
              </span>
              <span className="shrink-0 text-sm text-muted-foreground">
                {couponTermsLabel(coupon)}
              </span>
            </li>
          ))}
        </ul>
      )}
      <div className="flex flex-wrap items-center gap-4">
        <AddRow onClick={() => navigate(newCouponPath)}>
          <Plus size={12} /> New coupon for this plan
        </AddRow>
        <Link
          to={`${basePath}/coupons`}
          className="ml-auto text-xs text-muted-foreground hover:text-foreground hover:underline"
        >
          {total > coupons.length
            ? `All coupons, ${total - coupons.length} more here`
            : 'All coupons'}
        </Link>
      </div>
    </div>
  )
}

/** "50% off · for 12 months"; a credit coupon has no duration. */
const couponTermsLabel = (coupon: Coupon) =>
  coupon.discount?.discountType.case === 'credit'
    ? couponDiscountLabel(coupon.discount)
    : `${couponDiscountLabel(coupon.discount)} · ${couponDurationLabel(coupon).toLowerCase()}`

/** The plan's other sections, each with whether it is set up and what it holds. */
const usePlanExtras = (): PlanExtra[] => {
  const { plan, version, trial } = usePlanWithVersion()
  const isDraft = useIsDraftVersion()
  const navigate = useNavigate()
  const basePath = useBasePath()
  const location = useLocation()
  const isExpress = useIsExpressOrganization()
  const canManage = usePermissions().has('catalog:manage')
  const addOns = useQuery(listAddOns, version ? { planVersionId: version.id } : skipToken).data
    ?.addOns
  const page = useQuery(getPlanPricingPage, plan ? { planId: plan.id } : skipToken).data?.page
  const coupons = usePlanCoupons(plan?.id, COUPONS_LISTED)
  const sellsLinks = !!plan && !isDraft && plan.planType !== PlanType.CUSTOM
  const links = useQuery(
    listPurchaseLinks,
    sellsLinks && plan ? { planId: plan.id, includeArchived: false } : skipToken
  ).data?.links
  // The modules' sections; the module list is fixed per build, so their hooks keep their order.
  const moduleExtras = slotItems('planDetail.extras').map(item =>
    item.useExtra({ planVersionId: version?.id })
  )
  if (!plan || !version) return []

  const free = plan.planType === PlanType.FREE
  const trialTerms = trial?.terms
  const highlights = page?.highlights.length ?? 0
  const newCouponPath = `${basePath}/coupons/add-coupon?planId=${plan.id}&returnTo=${encodeURIComponent(
    location.pathname
  )}`
  const extras: PlanExtra[] = [
    {
      id: 'trial',
      title: 'Free trial',
      subtitle: free
        ? 'Days with a paid plan’s features before continuing on this free plan.'
        : 'Days new subscribers get before billing starts. Live on every version.',
      icon: TimerIcon,
      configured: !!trialTerms,
      summary: trialTerms
        ? [
            `${trialTerms.durationDays} ${trialTerms.durationDays === 1 ? 'day' : 'days'}`,
            !free && !trialTerms.free && 'billed from day one',
            (trial?.plansSharing ?? 0) > 1 && `offer ${trial?.name}`,
          ]
            .filter(Boolean)
            .join(' · ')
        : undefined,
      pitch: 'Let new subscribers try it first',
      opens: 'popover',
      readOnly: !canManage,
      content: close => (
        <PlanTrial
          trial={trial}
          currentPlanId={plan.id}
          planType={plan.planType}
          readOnly={!canManage}
          onDone={close}
        />
      ),
    },
    {
      id: 'eligibility',
      title: 'Who can get it',
      subtitle: 'Live on every version, saved right away. Prices don’t change.',
      icon: UserCheckIcon,
      configured: !isOpen(plan.eligibility),
      summary: eligibilitySummary(plan.eligibility, code => getCountryName(code)),
      pitch: 'Only some customers, by country or a customer property',
      opens: 'popover',
      readOnly: !canManage,
      content: close => (
        <PlanEligibility planId={plan.id} eligibility={plan.eligibility} onDone={close} />
      ),
    },
    {
      id: 'addons',
      title: 'Add-ons',
      subtitle: 'Extras a subscriber on this version can add to their subscription.',
      icon: PuzzleIcon,
      configured: (addOns?.length ?? 0) > 0,
      summary: addOns?.length
        ? `${addOns.length} offered · ${addOns.map(a => a.name).join(', ')}`
        : undefined,
      pitch: 'Extras a subscriber can buy with it',
      // Opened with none yet, the section starts on the catalog.
      opens: 'panel',
      readOnly: !isDraft,
      content: <AddOnSection />,
    },
    {
      id: 'coupons',
      title: 'Coupons',
      subtitle: 'Live coupons that apply to this plan, including those for every plan.',
      icon: TicketIcon,
      configured: coupons.total > 0,
      summary: couponsSummary(coupons.featured, coupons.total),
      pitch: 'Discounts customers can redeem on it',
      opens: 'panel',
      onAdd: () => navigate(newCouponPath),
      content: (
        <PlanCoupons
          coupons={coupons.coupons}
          total={coupons.total}
          newCouponPath={newCouponPath}
        />
      ),
    },
    {
      id: 'pricing-page',
      title: 'Pricing page',
      subtitle: `What visitors read about ${plan.name}. Editing it never makes a new version.`,
      icon: LayoutTemplateIcon,
      configured: !!page && (page.popular || !!page.tagline || highlights > 0),
      summary: [
        page?.popular && 'Popular',
        highlights > 0 && `${highlights} ${highlights === 1 ? 'highlight' : 'highlights'}`,
        page?.tagline,
      ]
        .filter(Boolean)
        .join(' · '),
      pitch: 'Its card on the public pricing page',
      opens: 'sheet',
      readOnly: isExpress || !canManage,
      content: close => (
        <>
          {plan.selfServiceRank === undefined && (
            <p className="mb-4 flex items-center gap-2 rounded-md bg-muted/50 px-3 py-2 text-xs text-muted-foreground">
              <InfoLineIcon size={13} className="shrink-0" />
              Shown once the plan is on the pricing page: add it from the plans list.
            </p>
          )}
          <PlanPricingPageForm
            key={plan.id}
            planId={plan.id}
            planVersionId={plan.activeVersionId ?? version.id}
            readOnly={isExpress || !canManage}
            onCancel={close}
            onSaved={close}
            className="flex-1"
            actionsClassName="sticky bottom-0 -mx-6 mt-auto border-t border-border bg-background px-6 py-4 dark:bg-[#171717]"
          />
        </>
      ),
    },
  ]
  if (!free && (hasSlot('planDetail.minimumCommitment') || version.minimumCommitment)) {
    extras.push({
      id: 'minimum',
      title: 'Minimum commitment',
      subtitle: 'The least each subscription pays per billing period. Part of the version.',
      icon: TrendingUpIcon,
      configured: !!version.minimumCommitment,
      summary: version.minimumCommitment
        ? `${formatCurrency(BigInt(version.minimumCommitment.amountSubunit.toString()), version.currency)} per period`
        : undefined,
      pitch: 'Guarantee an amount per period',
      opens: 'popover',
      readOnly: !isDraft || !hasSlot('planDetail.minimumCommitment'),
      content: close =>
        hasSlot('planDetail.minimumCommitment') ? (
          <Slot name="planDetail.minimumCommitment" planVersion={version} onDone={close} />
        ) : (
          <PlanMinimumCommitmentDisplay planVersion={version} />
        ),
    })
  }
  if (sellsLinks) {
    extras.push({
      id: 'purchase-links',
      title: 'Purchase links',
      subtitle: 'Public links to buy this plan, no account needed.',
      icon: LinkIcon,
      configured: (links?.length ?? 0) > 0,
      summary: links?.length
        ? `${links.length} ${links.length === 1 ? 'link' : 'links'}`
        : undefined,
      pitch: 'Sell it from a link, no account needed',
      opens: 'panel',
      content: <PurchaseLinksSection />,
    })
  }
  // A published version is read: what it doesn't have is added in a draft.
  const versioned = new Set(['addons', 'minimum'])
  return [...extras, ...moduleExtras.filter(extra => extra !== undefined)].filter(
    extra => isDraft || extra.configured || !versioned.has(extra.id)
  )
}
