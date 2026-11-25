import { skipToken } from '@connectrpc/connect-query'
import { InfoIcon } from '@md/icons'
import { Alert, Button, Tabs, TabsContent, TabsList, TabsTrigger, cn } from '@md/ui'
import { PaginationState } from '@tanstack/react-table'
import { ScopeProvider } from 'jotai-scope'
import {
  AlertTriangleIcon,
  ExternalLinkIcon,
  InfoIcon as InfoLineIcon,
  LayoutTemplateIcon,
  LinkIcon,
  Plus,
  PuzzleIcon,
  TicketIcon,
  TimerIcon,
  TrendingUpIcon,
  UserCheckIcon,
} from 'lucide-react'
import { ReactNode, useRef, useState } from 'react'
import { Link, useLocation, useNavigate, useParams } from 'react-router'

import { InlineLoading } from '@/components/Loading'
import { Slot, hasSlot, slotItems } from '@/edition'
import { eligibilitySummary, isOpen } from '@/features/eligibility/eligibility'
import {
  ResolvedEntitlementsPanel,
  ResolvedEntitlementsPanelHandle,
} from '@/features/entitlements/resolved/ResolvedEntitlementsPanel'
import { ListPlanVersionTab } from '@/features/plans/ListPlanVersion'
import { AddRow, MoreForPlan } from '@/features/plans/MoreForPlan'
import { PlanActions } from '@/features/plans/PlanActions'
import { AddOnSection } from '@/features/plans/addons/AddOnSection'
import { LockedPrices } from '@/features/plans/canvas/LockedPrices'
import { SellingGapsBanner } from '@/features/plans/canvas/SellingGapsBanner'
import { useCreditPaidLabels } from '@/features/plans/creditPaidMetrics'
import { PlanOverview, PlanVersionFooter } from '@/features/plans/details/PlanDetails'
import { PlanEligibility } from '@/features/plans/eligibility/PlanEligibility'
import {
  useIsDraftVersion,
  usePlanOverview,
  usePlanWithVersion,
} from '@/features/plans/hooks/usePlan'
import { PlanMinimumCommitmentDisplay } from '@/features/plans/minimum/PlanMinimumCommitment'
import { PriceComponentSection } from '@/features/plans/pricecomponents/PriceComponentSection'
import {
  editedComponentsAtom,
  priceCellRequestAtom,
} from '@/features/plans/pricecomponents/utils'
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
  const { planVersion } = useParams<{ planVersion?: string }>()

  const isDraft = useIsDraftVersion()
  const overview = usePlanOverview()
  const { version } = usePlanWithVersion()
  const [tab, setTab] = useState('overview')

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

  return (
    <ScopeProvider atoms={[editedComponentsAtom, priceCellRequestAtom]}>
      <CreditRateOptionalContext.Provider value={rateOptional}>
        <div className="flex h-full w-full flex-col space-y-5">
          <PlanHeader onHistory={isDraft ? undefined : () => setTab('versions')} />

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
              {showDraftAlert && (
                <>
                  <Alert variant="brand" className="flex  gap-2 items-center">
                    <div>
                      <InfoIcon size={16} className="flex text-brand  " />
                    </div>
                    <span className="text-foreground text-xs grow flex gap-1">
                      A{' '}
                      <Link to={`../${overview.localId}/draft`} className="hover:underline">
                        draft version
                      </Link>{' '}
                      has unpublished changes
                    </span>
                    <div className="text-xs">
                      <Button size="sm" variant="link" className="h-fit" hasIcon asChild>
                        <Link to={`../${overview.localId}/draft`}>
                          Open draft <ExternalLinkIcon size="12" />
                        </Link>
                      </Button>
                    </div>
                  </Alert>
                </>
              )}
              <Tabs value={tab} onValueChange={setTab} className="w-full">
                <TabsList className="w-full justify-start">
                  <TabsTrigger value="overview">Details</TabsTrigger>
                  <TabsTrigger value="subscriptions">Subscriptions</TabsTrigger>
                  <TabsTrigger value="alerts">Alerts</TabsTrigger>
                  <TabsTrigger value="versions">History</TabsTrigger>
                </TabsList>
                <TabsContent value="overview">
                  <PlanBody />
                </TabsContent>
                <TabsContent value="subscriptions">
                  <SubscriptionsTab />
                </TabsContent>
                <TabsContent value="alerts">
                  <>Alerts are not implemented yet</>
                </TabsContent>
                <TabsContent value="versions">
                  <ListPlanVersionTab />
                </TabsContent>
              </Tabs>
            </>
          )}
        </div>
        {children}
      </CreditRateOptionalContext.Provider>
    </ScopeProvider>
  )
}

/** "Plans / Pro", the name, its version and where it stands, the version's actions. */
const PlanHeader = ({ onHistory }: { onHistory?: () => void }) => {
  const basePath = useBasePath()
  const overview = usePlanOverview()
  const { plan, version } = usePlanWithVersion()
  const isDraft = useIsDraftVersion()
  const name = overview?.name ?? plan?.name
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
  const archived = overview?.planStatus === PlanStatus.ARCHIVED
  const context = !version
    ? undefined
    : isDraft
      ? live
        ? [
            `from v${live.version}, published`,
            subscriptions > 0 &&
              `${subscriptions} ${subscriptions === 1 ? 'subscription stays' : 'subscriptions stay'} on v${live.version} until migrated`,
          ]
            .filter(Boolean)
            .join(' · ')
        : 'Never published'
      : rank !== undefined
        ? [
            ranked > 0 ? `Rank ${rank} of ${Math.max(ranked, rank)}` : `Rank ${rank}`,
            page?.popular && 'Popular',
          ]
            .filter(Boolean)
            .join(' · ')
        : !isLive && live
          ? `v${live.version} is the published one`
          : `${subscriptions} ${subscriptions === 1 ? 'subscription' : 'subscriptions'}`

  return (
    <header className="space-y-2">
      <nav aria-label="Breadcrumb" className="text-xs text-muted-foreground">
        <Link to={`${basePath}/plans`} className="hover:text-foreground hover:underline">
          Plans
        </Link>
        <span className="px-1.5">/</span>
        <span>{name}</span>
      </nav>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <h1 className="text-2xl font-semibold">{name}</h1>
        {version &&
          (isDraft ? (
            <span className="rounded-md bg-warning/15 px-2 py-0.5 text-xs font-semibold text-warning">
              Draft v{version.version}
            </span>
          ) : (
            <span
              className={cn(
                'rounded-md px-2 py-0.5 text-xs font-semibold',
                isLive && !archived
                  ? 'bg-success/15 text-success'
                  : 'bg-muted text-muted-foreground'
              )}
            >
              v{version.version} · {archived ? 'Archived' : isLive ? 'Published' : 'Earlier version'}
            </span>
          ))}
        {context && <span className="text-sm text-muted-foreground">{context}</span>}
        <div className="ml-auto flex items-center gap-3">
          <PlanActions onHistory={onHistory} />
        </div>
      </div>
    </header>
  )
}

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
    <div>
      {!isExpress && (
        <div className="flex py-2 justify-end">
          <Button
            variant="secondary"
            hasIcon
            onClick={() =>
              navigate(`${basePath}/subscriptions/create?planVersionId=${planData?.version?.id}`)
            }
          >
            <Plus size={10} /> New subscription
          </Button>
        </div>
      )}

      <SubscriptionsTable
        data={data}
        totalCount={count}
        pagination={pagination}
        setPagination={setPagination}
        isLoading={isLoading}
        hidePlan
      />
    </div>
  )
}

const EntitlementsSectionForPlanVersion = ({ planVersionId }: { planVersionId: string }) => {
  const panelRef = useRef<ResolvedEntitlementsPanelHandle>(null)
  const creditPaidLabels = useCreditPaidLabels(planVersionId)
  return (
    <section aria-labelledby="plan-entitlements" className="space-y-3 pb-8">
      <div className="flex items-center justify-between gap-3">
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
          <h2 id="plan-entitlements" className="text-lg font-semibold">
            Entitlements
          </h2>
          <span className="text-sm text-muted-foreground">
            From its products, unless set on the plan
          </span>
        </div>
        <Button variant="outline" size="sm" hasIcon onClick={() => panelRef.current?.openAdd()}>
          <Plus size={14} /> Entitlement
        </Button>
      </div>
      <ResolvedEntitlementsPanel
        ref={panelRef}
        entity={{ type: 'plan-version', id: planVersionId }}
        canPin={true}
        hideAddButton
        hidePinAll
        flat
        meteredLabels={creditPaidLabels}
      />
    </section>
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
    <div className="space-y-5">
      {priced && <SellingGapsBanner />}
      <PlanOverview plan={plan} version={current} />
      {priced && !isDraft && <LockedPrices />}
      {priced && isDraft && (
        <div id="pricing">
          <PriceComponentSection />
        </div>
      )}

      {env.entitlementsEnabled && <EntitlementsSectionForPlanVersion planVersionId={current.id} />}

      <MoreForPlan extras={extras} />

      <Slot name="planDetail.sections" planVersionId={current.id} />

      <PlanVersionFooter plan={plan} version={current} />
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
          {total > coupons.length ? `All coupons, ${total - coupons.length} more here` : 'All coupons'}
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
  const { plan, version } = usePlanWithVersion()
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
  const trialDays = version.trialConfig?.durationDays ?? 0
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
        : 'Days new subscribers get before billing starts. Part of the version.',
      icon: TimerIcon,
      configured: trialDays > 0,
      summary: [
        `${trialDays} ${trialDays === 1 ? 'day' : 'days'}`,
        !free && version.trialConfig?.trialIsFree === false && 'billed from day one',
      ]
        .filter(Boolean)
        .join(' · '),
      pitch: 'Let new subscribers try it first',
      opens: 'popover',
      readOnly: !isDraft,
      content: close => (
        <PlanTrial
          config={version.trialConfig}
          currentPlanId={plan.id}
          currentPlanVersionId={version.id}
          planType={plan.planType}
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
        <PlanCoupons coupons={coupons.coupons} total={coupons.total} newCouponPath={newCouponPath} />
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
  const versioned = new Set(['trial', 'addons', 'minimum'])
  return [...extras, ...moduleExtras.filter(extra => extra !== undefined)].filter(
    extra => isDraft || extra.configured || !versioned.has(extra.id)
  )
}
