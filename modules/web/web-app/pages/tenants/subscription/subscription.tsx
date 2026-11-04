import { skipToken, useMutation } from '@connectrpc/connect-query'
import {
  Button,
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
  Skeleton,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  cn,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { parseISO } from 'date-fns'
import {
  ArrowLeftRight,
  ChevronDown,
  ChevronLeftIcon,
  Clock,
  ExternalLink,
  InfoIcon,
  Pencil,
  Plus,
  SlidersHorizontal,
  TriangleAlert,
} from 'lucide-react'
import { ReactNode, useMemo, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { toast } from 'sonner'

import { CopyToClipboardButton } from '@/components/CopyToClipboard'
import { Slot, slotItems, useVisibleItems } from '@/edition'
import { EntityActivityTimeline } from '@/features/activity/EntityActivityTimeline'
import { AddCreditDialog } from '@/features/credit/AddCreditDialog'
import { WalletSettingsDialog } from '@/features/credit/WalletSettingsDialog'
import { walletTitle } from '@/features/credit/format'
import { paymentMethodLabel } from '@/features/credit/paymentMethods'
import { CustomPropertyValue } from '@/features/customproperties/CustomPropertyValue'
import { useCustomPropertyRows } from '@/features/customproperties/useCustomPropertyRows'
import { EffectiveEntitlementsCard } from '@/features/entitlements/customer/EffectiveEntitlementsCard'
import {
  IntegrationType,
  SyncSubscriptionModal,
} from '@/features/settings/integrations/SyncSubscriptionModal'
import { CancelSubscriptionModal } from '@/features/subscriptions/CancelSubscriptionModal'
import { EditSubscriptionModal } from '@/features/subscriptions/EditSubscriptionModal'
import { FeatureUsageList, byUsage } from '@/features/subscriptions/FeatureUsageList'
import { SlotTransactionsModal } from '@/features/subscriptions/SlotTransactionsModal'
import { SubscriptionAttention } from '@/features/subscriptions/SubscriptionAttention'
import { SubscriptionCreditSummary } from '@/features/subscriptions/SubscriptionCreditSummary'
import { SubscriptionInvoicesTable } from '@/features/subscriptions/SubscriptionInvoicesTable'
import { SubscriptionMinimumProgress } from '@/features/subscriptions/SubscriptionMinimumProgress'
import { SubscriptionNextInvoice } from '@/features/subscriptions/SubscriptionNextInvoice'
import { SubscriptionPricingTerms } from '@/features/subscriptions/SubscriptionPricingTerms'
import { SubscriptionSectionCard } from '@/features/subscriptions/SubscriptionSectionCard'
import { useSubscriptionUsage } from '@/features/subscriptions/SubscriptionUsageSection'
import { UpdateSlotModal } from '@/features/subscriptions/UpdateSlotModal'
import {
  USAGE_FILTER_NONE_HELP,
  formatUsageFilter,
  formatUsageFilterSince,
} from '@/features/subscriptions/UsageFilterFields'
import { invalidateSubscriptionBilling } from '@/features/subscriptions/invalidateSubscriptionBilling'
import {
  issueDay,
  nextInvoiceAmount,
  nextInvoiceTitle,
  outstandingFact,
  overdueDetails,
  primaryAction,
} from '@/features/subscriptions/subscriptionOverview'
import {
  summarizeSubscriptionPrice,
  usagePriceRanges,
} from '@/features/subscriptions/subscriptionPrice'
import { subscriptionWallets } from '@/features/subscriptions/subscriptionWallets'
import { useSlotCounts, useUsageUnits } from '@/features/subscriptions/useUsageUnits'
import { formatDateRange, formatShortDay } from '@/features/subscriptions/utils/dates'
import { useBasePath } from '@/hooks/useBasePath'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { usePermissions } from '@/hooks/usePermissions'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { formatCurrency } from '@/lib/utils/numbers'
import { getLatestConnMeta } from '@/pages/tenants/utils'
import { listConnectors } from '@/rpc/api/connectors/v1/connectors-ConnectorsService_connectquery'
import { ConnectorProviderEnum } from '@/rpc/api/connectors/v1/models_pb'
import { getCustomerById } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { CustomPropertyEntityType } from '@/rpc/api/customproperties/v1/models_pb'
import { getEffectiveEntitlements } from '@/rpc/api/entitlements/v1/entitlements-EntitlementsService_connectquery'
import { PlanType } from '@/rpc/api/plans/v1/models_pb'
import {
  getPlanWithVersionByVersionId,
  listPlans,
} from '@/rpc/api/plans/v1/plans-PlansService_connectquery'
import {
  PaymentMethodsConfig,
  PendingScheduledEvent,
  ScheduledEventType,
  SubscriptionFee,
  SubscriptionStatus,
} from '@/rpc/api/subscriptions/v1/models_pb'
import {
  activateSubscription,
  cancelScheduledEvent,
  generateCheckoutToken,
  getSubscriptionDetails,
  getSubscriptionReceivables,
  getUpcomingInvoice,
  listSubscriptions,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { listCustomerWallets } from '@/rpc/api/wallets/v1/wallets-WalletsService_connectquery'
import { formatDateOnly, parseAndFormatDate } from '@/utils/date'
import { useTypedParams } from '@/utils/params'

import type { SlotUpdateData } from '@/features/subscriptions/SubscriptionPricingTerms'
import type { Tagged } from '@/features/subscriptions/subscriptionOverview'
import type { CreditUnitNames, PricedFee } from '@/features/subscriptions/subscriptionPrice'
import type { Subscription as SubscriptionModel } from '@/rpc/api/subscriptions/v1/models_pb'
import type { Wallet } from '@/rpc/api/wallets/v1/models_pb'

const OTHERS_SHOWN = 3

// Status Badge Component
const StatusBadge = ({ status }: { status: SubscriptionStatus }) => {
  const statusConfig = {
    [SubscriptionStatus.PENDING]: {
      bg: 'bg-warning/20',
      text: 'text-warning',
      render: 'pending',
    },
    [SubscriptionStatus.TRIALING]: {
      bg: 'bg-brand/20',
      text: 'text-brand',
      render: 'trialing',
    },
    [SubscriptionStatus.ACTIVE]: {
      bg: 'bg-success/20',
      text: 'text-success',
      render: 'active',
    },
    [SubscriptionStatus.CANCELED]: {
      bg: 'bg-destructive/20',
      text: 'text-destructive',
      render: 'cancelled',
    },
    [SubscriptionStatus.ENDED]: {
      bg: 'bg-muted/20',
      text: 'text-muted-foreground',
      render: 'ended',
    },
    [SubscriptionStatus.ABORTED]: {
      bg: 'bg-muted/20',
      text: 'text-muted-foreground',
      render: 'aborted',
    },
    [SubscriptionStatus.TRIAL_EXPIRED]: {
      bg: 'bg-warning/30',
      text: 'text-warning',
      render: 'trial expired',
    },
    [SubscriptionStatus.ERRORED]: {
      bg: 'bg-destructive/20',
      text: 'text-destructive',
      render: 'errored',
    },
  }

  const config = statusConfig[status]

  return (
    <span
      className={`inline-flex items-center rounded-full px-2 py-1 text-xs font-medium ${config.bg} ${config.text}`}
    >
      {config.render}
    </span>
  )
}

const paymentChannel = (config?: PaymentMethodsConfig): string => {
  switch (config?.config.case) {
    case 'bankTransfer':
      return 'bank transfer'
    case 'external':
      return 'paid outside Meteroid'
    default:
      return 'online payment'
  }
}

const SectionTitle = ({ children }: { children: ReactNode }) => (
  <h3 className="text-lg font-medium text-foreground mb-3">{children}</h3>
)

const DetailRow = ({
  label,
  value,
  link,
  externalLink,
}: {
  label: string
  value: ReactNode
  link?: string
  externalLink?: string
}) => (
  <div className="text-[13px] flex justify-between gap-3 py-2 border-b border-border last:border-0">
    <div className="text-muted-foreground">{label}</div>
    {externalLink && (
      <a href={externalLink} target="_blank" rel="noopener noreferrer">
        <div className="font-medium text-brand hover:underline">{value ?? 'N/A'}</div>
      </a>
    )}
    {link && (
      <Link to={link}>
        <div className="font-medium text-brand hover:underline">{value}</div>
      </Link>
    )}
    {!link && !externalLink && (
      <div className="text-right font-medium text-foreground">{value}</div>
    )}
  </div>
)

const DetailSection = ({ title, children }: { title: string; children: ReactNode }) => (
  <div className="mb-6">
    <SectionTitle>{title}</SectionTitle>
    <div className="space-y-1">{children}</div>
  </div>
)

/** When the subscription started, changes, renews or ends, with the undo of a cancellation. */
const TermSection = ({
  subscription,
  pendingEvents,
  canManage,
  onUndone,
}: {
  subscription: SubscriptionModel
  pendingEvents: PendingScheduledEvent[]
  canManage: boolean
  onUndone: () => void
}) => {
  const undo = useMutation(cancelScheduledEvent, {
    onSuccess: () => {
      toast.success('Cancellation undone: the subscription renews as usual')
      onUndone()
    },
    onError: (error: Error) => toast.error(`Failed to undo the cancellation: ${error.message}`),
  })
  const cancellation = pendingEvents.find(e => e.eventType === ScheduledEventType.CANCEL)
  const planChange = pendingEvents.find(e => e.eventType === ScheduledEventType.PLAN_CHANGE)
  const ends = cancellation?.scheduledDate ?? subscription.endDate
  const periodEnd = subscription.currentPeriodEnd

  return (
    <DetailSection title="Term">
      {subscription.endDate ? (
        <DetailRow
          label="Contract"
          value={formatDateRange(subscription.startDate, subscription.endDate)}
        />
      ) : (
        <DetailRow label="Started" value={formatDateOnly(subscription.startDate)} />
      )}
      {planChange && (
        <DetailRow
          label={`Changes to ${planChange.newPlanName ?? 'a new plan'}`}
          value={parseAndFormatDate(planChange.scheduledDate)}
        />
      )}
      {ends ? (
        <DetailRow
          label="Ends"
          value={`${parseAndFormatDate(ends)}${cancellation ? ' · cancellation pending' : ''}`}
        />
      ) : (
        periodEnd && (
          <DetailRow
            label="Renews"
            value={`${parseAndFormatDate(periodEnd)}${planChange ? ` · on ${planChange.newPlanName}` : ''}`}
          />
        )
      )}
      {cancellation && canManage && (
        <Button
          size="sm"
          variant="outline"
          className="mt-2 w-full"
          disabled={undo.isPending}
          onClick={() => undo.mutate({ eventId: cancellation.id, subscriptionId: subscription.id })}
        >
          Undo cancellation
        </Button>
      )}
    </DetailSection>
  )
}

const meteredMetricIds = (fee?: SubscriptionFee): string[] => {
  const f = fee?.fee
  return f && (f.case === 'usage' || f.case === 'capacity') ? [f.value.metricId] : []
}

const slotUnitsOf = (fee?: SubscriptionFee): string[] => {
  const f = fee?.fee
  return f && f.case === 'slot' ? [f.value.unit] : []
}

const LIVE_STATUSES = [SubscriptionStatus.ACTIVE, SubscriptionStatus.TRIALING]

const Fact = ({ label, children }: { label: string; children: ReactNode }) => (
  <div className="flex min-w-0 flex-col gap-1">
    <dt className="text-xs text-muted-foreground">{label}</dt>
    <dd className="text-sm font-medium text-foreground">{children}</dd>
  </div>
)

const OutstandingValue = ({
  fact,
  details,
  to,
}: {
  fact: Tagged
  details: string[]
  to?: string
}) => {
  const className = cn(
    fact.tone === 'destructive' && 'text-destructive',
    details.length > 0 &&
      'underline decoration-muted-foreground/40 decoration-dotted underline-offset-[3px]',
    to ? 'hover:underline hover:decoration-solid' : details.length > 0 && 'cursor-help'
  )
  const value = to ? (
    <Link to={to} className={className}>
      {fact.text}
    </Link>
  ) : (
    <span className={className}>{fact.text}</span>
  )
  if (details.length === 0) return value
  return (
    <Tooltip>
      <TooltipTrigger asChild>{value}</TooltipTrigger>
      <TooltipContent className="flex max-w-xs flex-col gap-1">
        {details.map(line => (
          <span key={line}>{line}</span>
        ))}
      </TooltipContent>
    </Tooltip>
  )
}

export const Subscription = () => {
  const navigate = useNavigate()
  const basePath = useBasePath()
  const isExpress = useIsExpressOrganization()
  const { has } = usePermissions()
  const canManageSubscriptions = has('billing:subscriptions:manage')
  const canManageCredit = has('billing:invoices:manage')
  const canViewInvoices = has('billing:invoices:read')

  const [showSyncHubspotModal, setShowSyncHubspotModal] = useState(false)
  const [showCancelModal, setShowCancelModal] = useState(false)
  const [showEditModal, setShowEditModal] = useState(false)
  const [slotUpdateData, setSlotUpdateData] = useState<SlotUpdateData | null>(null)
  const [slotTransactionsUnit, setSlotTransactionsUnit] = useState<string | null>(null)
  const [walletAction, setWalletAction] = useState<{
    action: 'add' | 'configure'
    wallet: Wallet
  } | null>(null)

  const { subscriptionId } = useTypedParams()
  const subscriptionQuery = useQuery(
    getSubscriptionDetails,
    { subscriptionId: subscriptionId ?? '' },
    { enabled: Boolean(subscriptionId) }
  )

  const details = subscriptionQuery.data
  const data = details?.subscription
  const moduleContext = { customerId: data?.customerId ?? '', subscriptionId: data?.id ?? '' }
  const moduleActions = useVisibleItems(slotItems('subscriptionDetail.actions'), moduleContext)
  const [openModuleAction, setOpenModuleAction] = useState<string>()
  const OpenModuleDialog = moduleActions.find(action => action.id === openModuleAction)?.Dialog

  const customPropertyRows = useCustomPropertyRows(
    CustomPropertyEntityType.SUBSCRIPTION,
    data?.customProperties
  )

  const planVersionQuery = useQuery(
    getPlanWithVersionByVersionId,
    { localId: data?.planVersionId ?? '' },
    { enabled: Boolean(data?.planVersionId) }
  )
  const plansQuery = useQuery(listPlans, {})
  const queryClient = useQueryClient()

  const planTrialConfig = planVersionQuery.data?.plan?.version?.trialConfig
  const planType = planVersionQuery.data?.plan?.plan?.planType
  const isFreePlan = planType === PlanType.FREE

  const trialingPlanName = planTrialConfig?.trialingPlanId
    ? plansQuery.data?.plans.find(p => p.id === planTrialConfig.trialingPlanId)?.name
    : undefined

  const connectorsQuery = useQuery(listConnectors, {})
  const isHubspotConnected = (connectorsQuery.data?.connectors ?? []).some(
    connector => connector.provider === ConnectorProviderEnum.HUBSPOT
  )
  const isLoading = subscriptionQuery.isLoading || connectorsQuery.isLoading
  const hubspotConnMeta = getLatestConnMeta(data?.connectionMetadata?.hubspot)

  const customerId = data?.customerId
  const live = !!data && LIVE_STATUSES.includes(data.status)
  const customerQuery = useQuery(getCustomerById, customerId ? { id: customerId } : skipToken)
  const walletsQuery = useQuery(listCustomerWallets, customerId ? { customerId } : skipToken)
  const receivablesQuery = useQuery(
    getSubscriptionReceivables,
    data && canViewInvoices ? { subscriptionId: data.id } : skipToken
  )
  const upcomingQuery = useQuery(
    getUpcomingInvoice,
    { subscriptionId: data?.id ?? '' },
    { enabled: live && Boolean(data?.currentPeriodStart) }
  )
  const usageQuery = useSubscriptionUsage(live ? data?.id : undefined)
  const othersQuery = useQuery(
    listSubscriptions,
    customerId
      ? { customerId, status: LIVE_STATUSES, pagination: { perPage: 20, page: 0 } }
      : skipToken
  )
  const entitlementsQuery = useQuery(
    getEffectiveEntitlements,
    { customerId: customerId ?? '' },
    { enabled: env.entitlementsEnabled && Boolean(customerId) }
  )

  const fees = useMemo<PricedFee[]>(
    () => [
      ...(details?.priceComponents ?? []).map(c => ({ fee: c.fee, period: c.period })),
      ...(details?.addOns ?? []).map(a => ({ fee: a.fee, period: a.period, quantity: a.quantity })),
    ],
    [details?.priceComponents, details?.addOns]
  )
  const metricIds = useMemo(() => fees.flatMap(f => meteredMetricIds(f.fee)), [fees])
  const slotUnits = useMemo(() => fees.flatMap(f => slotUnitsOf(f.fee)), [fees])
  const { units: usageUnits, isLoading: usageUnitsLoading } = useUsageUnits(metricIds)
  const slotCounts = useSlotCounts(data?.id ?? '', slotUnits)

  const allWallets = walletsQuery.data?.wallets ?? []
  const wallets = data ? subscriptionWallets(allWallets, data.currency, fees) : []
  const currencyWallets = wallets.filter(w => !w.creditUnit)
  const creditUnits: Record<string, CreditUnitNames> = Object.fromEntries(
    allWallets.flatMap((w): [string, CreditUnitNames][] =>
      w.creditUnit ? [[w.creditUnit.id, w.creditUnit]] : []
    )
  )
  const minimumSubunit = details?.minimumCommitment?.commitment?.amountSubunit
  const customer = customerQuery.data?.customer
  const currentPaymentMethod = customer?.paymentMethods.find(
    m => m.id === customer.currentPaymentMethodId
  )
  const paymentMethod = currentPaymentMethod ? paymentMethodLabel(currentPaymentMethod) : undefined
  const others = (othersQuery.data?.subscriptions ?? []).filter(s => s.id !== data?.id)
  const receivables = receivablesQuery.data?.receivables

  const canCancelSubscription =
    !!data &&
    data.status !== SubscriptionStatus.CANCELED &&
    data.status !== SubscriptionStatus.ENDED &&
    data.status !== SubscriptionStatus.ABORTED &&
    data.status !== SubscriptionStatus.ERRORED

  const checkoutTokenMutation = useMutation(generateCheckoutToken, {
    onSuccess: data => {
      window.open(`${window.location.origin}/checkout?token=${data.token}`, '_blank')
    },
    onError: error => toast.error(`Failed to open checkout: ${error.message}`),
  })

  const activateSubscriptionMutation = useMutation(activateSubscription, {
    onSuccess: async () => {
      if (subscriptionId) await invalidateSubscriptionBilling(queryClient, subscriptionId)
    },
    onError: error => toast.error('Failed to activate subscription: ' + error.message),
  })

  if (isLoading || !data || !details) {
    return (
      <div className="p-6">
        <Skeleton height={16} width={50} className="mb-4" />
        <div className="flex gap-6">
          <div className="flex-1">
            <Skeleton height={100} className="mb-4" />
            <Skeleton height={200} className="mb-4" />
          </div>
          <div className="w-80">
            <Skeleton height={300} className="mb-4" />
          </div>
        </div>
      </div>
    )
  }

  const priceSummary = summarizeSubscriptionPrice({
    currency: data.currency,
    fees,
    minimumSubunit,
    slotCounts,
    creditUnits,
    prepaid: slotItems('subscriptionCredit.live').some(live =>
      live.prepaid(data.id, currencyWallets)
    ),
  })
  const usagePricing =
    metricIds.length > 0
      ? usagePriceRanges({ currency: data.currency, fees, units: usageUnits, creditUnits })
      : []

  const cancellation = details.pendingEvents.find(e => e.eventType === ScheduledEventType.CANCEL)
  const trialing = data.status === SubscriptionStatus.TRIALING
  // A free trial ends on its EndTrial event, else with the period it runs as.
  const freeTrialEndDate =
    trialing && details.trialConfig?.isFree
      ? (details.pendingEvents.find(e => e.eventType === ScheduledEventType.END_TRIAL)
          ?.scheduledDate ?? data.currentPeriodEnd)
      : undefined
  const freeTrialEnd = freeTrialEndDate ? parseISO(freeTrialEndDate) : undefined
  const ended =
    data.status === SubscriptionStatus.CANCELED ||
    data.status === SubscriptionStatus.ENDED ||
    data.status === SubscriptionStatus.ABORTED
  const range =
    data.currentPeriodStart && data.currentPeriodEnd
      ? formatDateRange(data.currentPeriodStart, data.currentPeriodEnd)
      : undefined
  const currentPeriod = ended
    ? `Ended ${formatDateOnly(data.endDate ?? data.currentPeriodEnd ?? data.startDate)}`
    : data.status === SubscriptionStatus.PENDING
      ? data.pendingCheckout
        ? 'Starts at checkout'
        : `Starts ${formatDateOnly(data.billingStartDate ?? data.startDate)}`
      : range
        ? `${trialing ? 'Trial · ' : ''}${range}${cancellation ? ' · ends' : ''}`
        : undefined

  const upcoming = upcomingQuery.data?.invoice
  const money = (minor: bigint) => formatCurrency(minor, data.currency)
  const nextInvoiceFact = (): ReactNode => {
    if (!live || !data.currentPeriodStart) return undefined
    if (upcomingQuery.isLoading) return <Skeleton height={16} width={120} />
    if (!upcoming) return '—'
    if (upcoming.notInvoiced) return 'None: ends without an invoice'
    if (isFreePlan && upcoming.amountDue === 0n) return 'No charge'
    const parts = [
      nextInvoiceAmount(upcoming, money),
      formatShortDay(issueDay(upcoming)),
      !data.chargeAutomatically && upcoming.dueDate
        ? `due ${formatShortDay(upcoming.dueDate)}`
        : undefined,
    ]
    return parts.filter(Boolean).join(' · ')
  }
  const nextInvoice = nextInvoiceFact()
  const nextInvoiceLabel = upcoming ? nextInvoiceTitle(upcoming, trialing).title : 'Next invoice'

  const paymentTerms = data.chargeAutomatically ? (
    customerQuery.isLoading ? (
      <Skeleton height={16} width={160} />
    ) : paymentMethod ? (
      `Automatic · ${paymentMethod}`
    ) : (
      <Tooltip>
        <TooltipTrigger asChild>
          <span className="inline-flex cursor-help items-center gap-1.5">
            <TriangleAlert className="h-3.5 w-3.5 shrink-0 text-warning" />
            Automatic · no payment method
          </span>
        </TooltipTrigger>
        <TooltipContent className="max-w-xs">
          Invoices are charged automatically, but this customer has no payment method on file, so
          they stay unpaid until one is added.
        </TooltipContent>
      </Tooltip>
    )
  ) : (
    [
      'Invoice',
      data.netTerms > 0 ? `net ${data.netTerms}` : 'due on receipt',
      paymentChannel(data.paymentMethodsConfig),
    ].join(' · ')
  )
  const outstanding = outstandingFact(receivables)
  const pricing =
    usagePricing === undefined ? (
      usageUnitsLoading ? (
        <Skeleton height={16} width={140} />
      ) : undefined
    ) : usagePricing.length > 0 ? (
      usagePricing.join(' · ')
    ) : undefined

  const canAct = !isExpress
  const canManageWallets = canManageCredit && canAct
  const creditableWallets = canManageWallets ? wallets : []
  const action = canAct
    ? primaryAction({
        status: data.status,
        pendingCheckout: data.pendingCheckout && Boolean(data.checkoutUrl),
        hasWallet: creditableWallets.length > 0,
        isFreePlan: isFreePlan && canManageSubscriptions,
        hasMinimum: Boolean(details.minimumCommitment) && canManageSubscriptions,
      })
    : undefined

  const planFeatures = byUsage(usageQuery.data?.usage ?? [])
  const showUsageLimits = live && planFeatures.length > 0
  const entitlementCount = entitlementsQuery.data?.entitlements.length ?? 0
  const refetchAll = () => {
    void invalidateSubscriptionBilling(queryClient, data.id)
  }

  const primaryButton = (() => {
    switch (action) {
      case 'checkout':
        return data.checkoutUrl ? (
          <CopyToClipboardButton text="Copy checkout link" textToCopy={data.checkoutUrl} />
        ) : null
      case 'add-credit':
        return creditableWallets.length === 1 ? (
          <Button
            variant="primary"
            size="sm"
            hasIcon
            onClick={() => setWalletAction({ action: 'add', wallet: creditableWallets[0] })}
          >
            <Plus size={14} /> Add credit
          </Button>
        ) : (
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="primary" size="sm" hasIcon>
                <Plus size={14} /> Add credit <ChevronDown className="h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              {creditableWallets.map(wallet => (
                <DropdownMenuItem
                  key={wallet.id}
                  onClick={() => setWalletAction({ action: 'add', wallet })}
                >
                  {walletTitle(wallet)}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        )
      case 'change-plan':
        return (
          <Button variant="primary" size="sm" onClick={() => navigate('change-plan')}>
            Change plan
          </Button>
        )
      case 'amend':
        return (
          <Button variant="primary" size="sm" onClick={() => navigate('amend')}>
            Amend
          </Button>
        )
      default:
        return null
    }
  })()

  return (
    <div className="flex min-h-screen bg-background gap-2">
      {showSyncHubspotModal && (
        <SyncSubscriptionModal
          customerName={data.customerName}
          id={data.id}
          integrationType={IntegrationType.Hubspot}
          onClose={() => setShowSyncHubspotModal(false)}
        />
      )}
      {showCancelModal && (
        <CancelSubscriptionModal
          subscriptionId={data.id}
          customerName={data.customerName}
          planName={data.planName}
          onClose={() => setShowCancelModal(false)}
          onSuccess={refetchAll}
        />
      )}
      {showEditModal && (
        <EditSubscriptionModal
          subscription={data}
          usageFilterEditable={!ended && metricIds.length > 0}
          onClose={() => setShowEditModal(false)}
          onSuccess={refetchAll}
        />
      )}
      {walletAction?.action === 'add' && (
        <AddCreditDialog
          open
          onOpenChange={() => setWalletAction(null)}
          wallet={walletAction.wallet}
          customerCurrency={customer?.currency ?? data.currency}
          paymentMethods={customer?.paymentMethods ?? []}
          defaultPaymentMethodId={customer?.currentPaymentMethodId}
        />
      )}
      {walletAction?.action === 'configure' && (
        <WalletSettingsDialog
          wallet={walletAction.wallet}
          customerCurrency={customer?.currency ?? data.currency}
          paymentMethods={customer?.paymentMethods ?? []}
          onClose={() => setWalletAction(null)}
        />
      )}
      <div className="flex-1 min-w-0 p-6 pr-0">
        <section
          aria-label="Subscription summary"
          className="mb-6 rounded-lg border border-border bg-card p-5 shadow-sm"
        >
          <div className="flex w-full flex-wrap items-start justify-between gap-4">
            <div className="min-w-0">
              <div className="flex items-center">
                <ChevronLeftIcon
                  className="cursor-pointer text-muted-foreground hover:text-foreground mr-2"
                  onClick={() => navigate('..')}
                  size={20}
                />
                <Link
                  to={`${basePath}/plans/${data.planId}/${data.version}`}
                  className="text-xl font-semibold text-foreground hover:text-brand"
                >
                  {data.planName}
                </Link>
                <div className="ml-2">
                  <StatusBadge status={data.status} />
                </div>
              </div>
              <p className="mt-1 text-sm text-muted-foreground">
                <span className="font-semibold text-foreground">{priceSummary.headline}</span>
                {priceSummary.details.map(detail => ` · ${detail}`).join('')}
              </p>
            </div>
            {canAct && (
              <div className="flex items-center gap-2">
                {primaryButton}
                {canManageSubscriptions && (
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button variant="outline" className="gap-2" size="sm" hasIcon>
                        Actions <ChevronDown className="w-4 h-4" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      {!data.activatedAt && (
                        <DropdownMenuItem
                          onClick={() =>
                            activateSubscriptionMutation.mutate({ subscriptionId: data.id })
                          }
                          disabled={activateSubscriptionMutation.isPending}
                        >
                          <Clock size="16" className="mr-2" />
                          {activateSubscriptionMutation.isPending
                            ? 'Activating...'
                            : 'Activate Subscription'}
                        </DropdownMenuItem>
                      )}
                      {data.pendingCheckout && (
                        <DropdownMenuItem
                          onClick={() => checkoutTokenMutation.mutate({ subscriptionId: data.id })}
                          disabled={checkoutTokenMutation.isPending}
                        >
                          <ExternalLink size="16" className="mr-2" />
                          Open Checkout
                        </DropdownMenuItem>
                      )}
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <span>
                            <DropdownMenuItem
                              disabled={!isHubspotConnected}
                              onClick={() => setShowSyncHubspotModal(true)}
                            >
                              Sync To Hubspot
                            </DropdownMenuItem>
                          </span>
                        </TooltipTrigger>
                        {!isHubspotConnected && (
                          <TooltipContent>Hubspot integration not connected</TooltipContent>
                        )}
                      </Tooltip>
                      <DropdownMenuItem onClick={() => setShowEditModal(true)}>
                        <Pencil size="16" className="mr-2" />
                        Edit Billing Settings
                      </DropdownMenuItem>
                      {moduleActions.map(action => (
                        <DropdownMenuItem
                          key={action.id}
                          onClick={() => setOpenModuleAction(action.id)}
                        >
                          <action.icon size="16" className="mr-2" />
                          {action.label}
                        </DropdownMenuItem>
                      ))}
                      {live && (
                        <DropdownMenuItem onClick={() => navigate('change-plan')}>
                          <ArrowLeftRight size="16" className="mr-2" />
                          Change Plan
                        </DropdownMenuItem>
                      )}
                      {live && (
                        <DropdownMenuItem onClick={() => navigate('amend')}>
                          <SlidersHorizontal size="16" className="mr-2" />
                          Amend subscription
                        </DropdownMenuItem>
                      )}
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <span>
                            <DropdownMenuItem
                              disabled={!canCancelSubscription}
                              onClick={() => setShowCancelModal(true)}
                              className="text-destructive focus:text-destructive"
                            >
                              Cancel Subscription
                            </DropdownMenuItem>
                          </span>
                        </TooltipTrigger>
                        {!canCancelSubscription && (
                          <TooltipContent>
                            Subscription is already cancelled or ended
                          </TooltipContent>
                        )}
                      </Tooltip>
                    </DropdownMenuContent>
                  </DropdownMenu>
                )}
              </div>
            )}
          </div>
          <dl className="mt-4 grid grid-cols-2 gap-4 border-t border-border pt-4 md:grid-cols-3 xl:grid-cols-5">
            {currentPeriod && <Fact label="Current period">{currentPeriod}</Fact>}
            {nextInvoice !== undefined && <Fact label={nextInvoiceLabel}>{nextInvoice}</Fact>}
            <Fact label="Payment">{paymentTerms}</Fact>
            {outstanding && (
              <Fact label="Outstanding">
                <OutstandingValue
                  fact={outstanding}
                  details={overdueDetails(receivables, new Date())}
                  to={
                    receivables && receivables.overdueAmount > 0n
                      ? `${basePath}/invoices?subscription=${encodeURIComponent(data.localId)}&overdue=true`
                      : undefined
                  }
                />
              </Fact>
            )}
            {pricing && <Fact label="Pricing">{pricing}</Fact>}
          </dl>
        </section>

        <SubscriptionAttention
          subscription={data}
          pendingEvents={details.pendingEvents}
          wallets={wallets}
          featureUsage={live ? planFeatures : []}
          firstInvoiceAmount={trialing ? upcoming?.amountDue : undefined}
          hasPaymentMethod={Boolean(currentPaymentMethod)}
          freePlan={isFreePlan}
          canManageSubscription={canManageSubscriptions && canAct}
          canManageWallets={canManageWallets}
          onAddCredit={wallet => setWalletAction({ action: 'add', wallet })}
          onConfigureWallet={wallet => setWalletAction({ action: 'configure', wallet })}
          onEditBilling={() => setShowEditModal(true)}
          onChangePlan={() => navigate('change-plan')}
          onRefetch={refetchAll}
        />

        <SubscriptionCreditSummary
          subscription={data}
          wallets={wallets}
          fees={fees}
          creditUnits={creditUnits}
          pendingCredit={receivables?.pendingCredit ?? []}
          others={others}
          canAddCredit={canManageWallets}
          onAddCredit={wallet => setWalletAction({ action: 'add', wallet })}
        />

        <SubscriptionPricingTerms
          details={details}
          isFreePlan={isFreePlan}
          canManage={canManageSubscriptions && canAct}
          contractTerm={data.endDate ? formatDateRange(data.startDate, data.endDate) : undefined}
          paymentTerms={paymentTerms}
          onAmend={canManageSubscriptions && canAct && live ? () => navigate('amend') : undefined}
          onUpdateSlot={setSlotUpdateData}
          onViewTransactions={setSlotTransactionsUnit}
        />

        {details.minimumCommitment && live && (
          <SubscriptionMinimumProgress subscriptionId={data.id} />
        )}

        {live && data.currentPeriodStart && (
          <SubscriptionNextInvoice
            subscription={data}
            components={details.priceComponents}
            addOns={details.addOns}
            paymentMethod={paymentMethod}
            wallets={wallets}
            hasMinimum={Boolean(details.minimumCommitment)}
            hasMeteredFees={metricIds.length > 0}
            hideWhenFree={isFreePlan}
            units={usageUnits}
            creditUnits={creditUnits}
          />
        )}

        {showUsageLimits && (
          <SubscriptionSectionCard
            title="Usage limits"
            aside={<Slot name="subscriptionDetail.usageLimitsAside" {...moduleContext} />}
          >
            <div className="flex flex-col gap-4 p-4">
              <FeatureUsageList
                usage={planFeatures}
                elapsedPct={usageQuery.data?.cycle?.elapsedPct}
                freeTrialEnd={freeTrialEnd}
              />
            </div>
          </SubscriptionSectionCard>
        )}

        <SubscriptionInvoicesTable
          subscriptionLocalId={data.localId}
          chargeAutomatically={data.chargeAutomatically}
          hasMinimum={Boolean(details.minimumCommitment)}
          firstInvoiceDate={live ? (upcoming?.invoiceDate ?? data.currentPeriodEnd) : undefined}
        />

        {slotUpdateData && (
          <UpdateSlotModal
            subscriptionId={data.id}
            priceComponentId={slotUpdateData.priceComponentId}
            unit={slotUpdateData.unit}
            currentSlots={slotUpdateData.currentSlots}
            unitRate={slotUpdateData.unitRate}
            currency={data.currency}
            minSlots={slotUpdateData.minSlots}
            maxSlots={slotUpdateData.maxSlots}
            onClose={() => setSlotUpdateData(null)}
            onSuccess={() => {
              setSlotUpdateData(null)
              refetchAll()
            }}
          />
        )}
        {slotTransactionsUnit && (
          <SlotTransactionsModal
            subscriptionId={data.id}
            unit={slotTransactionsUnit}
            open={true}
            onClose={() => setSlotTransactionsUnit(null)}
          />
        )}

        {env.entitlementsEnabled && entitlementCount > 0 && (
          <Collapsible className="mb-6 rounded-lg border border-border bg-card shadow-sm">
            <CollapsibleTrigger className="group flex w-full items-center justify-between p-4 text-left">
              <h3 className="text-md font-medium text-foreground">
                Entitlements · {entitlementCount} {entitlementCount === 1 ? 'feature' : 'features'}
              </h3>
              <ChevronDown className="h-4 w-4 text-muted-foreground transition-transform group-data-[state=open]:rotate-180" />
            </CollapsibleTrigger>
            <CollapsibleContent className="border-t border-border p-4">
              <EffectiveEntitlementsCard
                customerId={data.customerId}
                currentSubscriptionId={data.id}
              />
            </CollapsibleContent>
          </Collapsible>
        )}

        {OpenModuleDialog && (
          <OpenModuleDialog {...moduleContext} onClose={() => setOpenModuleAction(undefined)} />
        )}
        <Slot name="subscriptionDetail.sections" {...moduleContext} />
      </div>

      <div className="w-80 shrink-0 p-6 border-l border-border">
        <DetailSection title="Customer">
          <DetailRow
            label="Customer"
            value={data.customerName}
            link={`${basePath}/customers/${data.customerId}`}
          />
          {data.customerAlias && <DetailRow label="Alias" value={data.customerAlias} />}
          {others.length === 0 ? (
            <DetailRow
              label="Other subscriptions"
              value={
                <span className="text-muted-foreground">
                  {othersQuery.isLoading ? '…' : 'None'}
                </span>
              }
            />
          ) : (
            <>
              {others.slice(0, OTHERS_SHOWN).map((other, i) => (
                <DetailRow
                  key={other.id}
                  label={i === 0 ? 'Other subscriptions' : ''}
                  value={
                    <span className="inline-flex items-center gap-2">
                      {other.planName}
                      <StatusBadge status={other.status} />
                    </span>
                  }
                  link={`${basePath}/subscriptions/${other.id}`}
                />
              ))}
              {others.length > OTHERS_SHOWN && (
                <DetailRow
                  label=""
                  value={`+${others.length - OTHERS_SHOWN} more`}
                  link={`${basePath}/customers/${data.customerId}`}
                />
              )}
            </>
          )}
        </DetailSection>

        <TermSection
          subscription={data}
          pendingEvents={details.pendingEvents}
          canManage={canManageSubscriptions && canAct}
          onUndone={refetchAll}
        />

        {data.trialDuration && (
          <DetailSection title="Trial Information">
            <DetailRow label="Trial Duration" value={`${data.trialDuration} days`} />
            {(data.status === SubscriptionStatus.TRIALING ||
              data.status === SubscriptionStatus.TRIAL_EXPIRED) && (
              <DetailRow label="Trial Status" value={<StatusBadge status={data.status} />} />
            )}
            {planTrialConfig && (
              <>
                <DetailRow
                  label="Trial Type"
                  value={
                    isFreePlan
                      ? 'Free plan'
                      : planTrialConfig.trialIsFree
                        ? 'Free trial'
                        : 'Paid trial'
                  }
                />
                {trialingPlanName && (
                  <DetailRow label="Trial Features" value={`"${trialingPlanName}" features`} />
                )}
              </>
            )}
          </DetailSection>
        )}

        <DetailSection title="Subscription Details">
          <DetailRow label="ID" value={data.localId} />
          <DetailRow label="Version" value={data.version} />
          <DetailRow label="Currency" value={data.currency} />
        </DetailSection>

        <DetailSection title="Billing Information">
          <DetailRow label="Billing Day" value={data.billingDayAnchor} />
          <DetailRow
            label="Auto-advance invoices"
            value={data.autoAdvanceInvoices ? 'Yes' : 'No'}
          />
          {data.invoiceMemo && <DetailRow label="Invoice Memo" value={data.invoiceMemo} />}
          {data.invoiceThreshold && (
            <DetailRow label="Invoice Threshold" value={data.invoiceThreshold} />
          )}
          {data.purchaseOrder && <DetailRow label="Purchase Order" value={data.purchaseOrder} />}
          <DetailRow
            label="Usage filter"
            value={
              data.usageFilter ? (
                <span className="flex flex-col items-end">
                  <span className="font-mono break-all">{formatUsageFilter(data.usageFilter)}</span>
                  {data.usageFilterEffectiveFrom && (
                    <span className="text-xs font-normal text-muted-foreground">
                      Since {formatUsageFilterSince(data.usageFilterEffectiveFrom)}
                    </span>
                  )}
                </span>
              ) : (
                <span className="flex items-center gap-1.5">
                  None
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <InfoIcon className="h-3.5 w-3.5 cursor-help text-muted-foreground" />
                    </TooltipTrigger>
                    <TooltipContent className="max-w-80">{USAGE_FILTER_NONE_HELP}</TooltipContent>
                  </Tooltip>
                </span>
              )
            }
          />
        </DetailSection>

        <DetailSection title="Integrations">
          {hubspotConnMeta?.externalId ? (
            <DetailRow
              label="Hubspot ID"
              value={hubspotConnMeta?.externalId}
              externalLink={`https://app.hubspot.com/contacts/${hubspotConnMeta?.externalCompanyId}/deal/${hubspotConnMeta?.externalId}`}
            />
          ) : (
            <span className="text-xs font-medium text-muted-foreground">-</span>
          )}
        </DetailSection>

        {customPropertyRows.length > 0 && (
          <DetailSection title="Custom properties">
            {customPropertyRows.map(row => (
              <DetailRow
                key={row.key}
                label={row.label}
                value={<CustomPropertyValue def={row.def} value={row.raw} />}
              />
            ))}
          </DetailSection>
        )}

        <DetailSection title="Activity">
          <EntityActivityTimeline
            entityType="subscription"
            entityId={data.id}
            emptyLabel="No activity yet for this subscription"
          />
        </DetailSection>
      </div>
    </div>
  )
}

export default Subscription
