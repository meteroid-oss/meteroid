import { timestampDate } from '@bufbuild/protobuf/wkt'
import { createConnectQueryKey } from '@connectrpc/connect-query'
import {
  Button,
  Card,
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Skeleton,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  cn,
} from '@md/ui'
import { useQueryClient } from '@tanstack/react-query'
import { formatDistanceToNow } from 'date-fns'
import { ChevronDown, Plus, RefreshCw, Settings2, WalletCards } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'

import { PanelEmptyState } from '@/components/empty-state/PanelEmptyState'
import { Slot, slotItems } from '@/edition'
import { BalanceLagNote } from '@/features/credit/AccessCheckNote'
import { AddCreditDialog } from '@/features/credit/AddCreditDialog'
import { CreateWalletDialog } from '@/features/credit/CreateWalletDialog'
import { CreditGrantsTable } from '@/features/credit/CreditGrantsTable'
import { CreditGroupsList } from '@/features/credit/CreditGroupsList'
import { CustomerLedgerTable } from '@/features/credit/CustomerLedgerTable'
import { RemoveCreditDialog } from '@/features/credit/RemoveCreditDialog'
import { WalletBalanceSummary } from '@/features/credit/WalletBalanceSummary'
import { WalletSettingsDialog, hasWalletSettings } from '@/features/credit/WalletSettingsDialog'
import { WalletSinceRow } from '@/features/credit/WalletSinceRow'
import { WalletUsageChart } from '@/features/credit/WalletUsageChart'
import { walletTitle } from '@/features/credit/format'
import { invalidateWallets } from '@/features/credit/labels'
import { dailyUsageTotal, walletDailyUsageInput } from '@/features/credit/usageChartData'
import { useLiveSubscriptions } from '@/features/credit/useLiveSubscriptions'
import { MAX_FUNDING_SUBSCRIPTIONS, useWalletPeriods } from '@/features/credit/useWalletPeriods'
import { periodCreditGranted, periodUtilization } from '@/features/credit/walletPeriodUsage'
import { PeriodUsageBar } from '@/features/subscriptions/PeriodUsageBar'
import { formatUsageFilter } from '@/features/subscriptions/UsageFilterFields'
import { useQuery } from '@/lib/connectrpc'
import { parseAndFormatDateTime } from '@/lib/utils/date'
import {
  getCustomerCreditActivity,
  listCustomerWallets,
  listWalletDailyUsageCost,
} from '@/rpc/api/wallets/v1/wallets-WalletsService_connectquery'

import type { CustomerWalletLive } from '@/edition/types'
import type { ActivityState } from '@/features/credit/CreditGrantsTable'
import type {
  FundingSubscription,
  WalletPeriod,
  WalletPeriodsStatus,
} from '@/features/credit/useWalletPeriods'
import type { CustomerPaymentMethod } from '@/rpc/api/customers/v1/models_pb'
import type {
  CreditGrant,
  GrantActivity,
  Wallet,
  WalletActivity,
} from '@/rpc/api/wallets/v1/models_pb'
import type { DescMethodUnary } from '@bufbuild/protobuf'

type Props = {
  customerId: string
  currency: string
  canManage: boolean
  paymentMethods: CustomerPaymentMethod[]
  defaultPaymentMethodId?: string
}

type Action = 'add' | 'configure'

// The ledger shows its latest movements; the whole statement opens in a dialog.
const LEDGER_PREVIEW_ROWS = 5

type WalletTab = 'activity' | 'grants' | 'summary'

const ALL_SUBSCRIPTIONS = '__all__'

// Usage figures the wallet queries do not cover, refetched with them.
const USAGE_QUERIES: DescMethodUnary[] = [listWalletDailyUsageCost]

// Without a module's live view, nothing is estimated: the ledger figures show.
const noScopedBalance: CustomerWalletLive['useScopedBalance'] = () => undefined
const noProjection: CustomerWalletLive['useProjection'] = () => undefined

export const CustomerWalletsPanel = ({
  customerId,
  currency,
  canManage,
  paymentMethods,
  defaultPaymentMethodId,
}: Props) => {
  const query = useQuery(listCustomerWallets, { customerId })
  // 'add-credit' opens the wallet for the credit chosen, then adds credit to it.
  const [creating, setCreating] = useState<'wallet' | 'add-credit' | null>(null)
  const [action, setAction] = useState<{ action: Action; wallet: Wallet } | null>(null)
  const [removal, setRemoval] = useState<{ wallet: Wallet; grant: CreditGrant } | null>(null)
  const [selectedId, setSelectedId] = useState<string>()
  const [scopeId, setScopeId] = useState<string>()

  // A wallet opened here is the one shown next.
  const knownWallets = useRef<Set<string> | undefined>(undefined)
  useEffect(() => {
    const ids = (query.data?.wallets ?? []).map(wallet => wallet.id)
    if (!query.data) return
    const opened = knownWallets.current && ids.find(id => !knownWallets.current?.has(id))
    if (opened) setSelectedId(opened)
    knownWallets.current = new Set(ids)
  }, [query.data])

  const { periods, status: periodsStatus } = useWalletPeriods(
    customerId,
    query.data?.wallets ?? [],
    !!query.data
  )
  const liveSubscriptions = useLiveSubscriptions(customerId)
  const liveSubscriptionCount = liveSubscriptions.length
  const scope =
    liveSubscriptionCount > 1 && liveSubscriptions.some(s => s.id === scopeId) ? scopeId : undefined
  // With a subscription chosen, the totals count the credit it can spend.
  const activityQuery = useQuery(getCustomerCreditActivity, { customerId, subscriptionId: scope })
  const grantActivity = useMemo(
    () =>
      new Map<string, GrantActivity>(
        (activityQuery.data?.wallets ?? []).flatMap(wallet =>
          wallet.grants.map(grant => [grant.creditGrantId, grant] as const)
        )
      ),
    [activityQuery.data]
  )
  const unbilledByGrant = useMemo(
    () =>
      new Map(
        [...grantActivity].map(([grantId, activity]) => [grantId, activity.unbilled] as const)
      ),
    [grantActivity]
  )
  const usageFilters = useMemo(
    () =>
      new Map(
        liveSubscriptions.flatMap(subscription =>
          subscription.usageFilter
            ? [[subscription.id, formatUsageFilter(subscription.usageFilter)] as const]
            : []
        )
      ),
    [liveSubscriptions]
  )
  const subscriptionLabel = (subscriptionId: string) => {
    const plan =
      liveSubscriptions.find(subscription => subscription.id === subscriptionId)?.planName ??
      'Subscription'
    const filter = usageFilters.get(subscriptionId)
    return filter ? `${plan} (${filter})` : plan
  }
  const queryClient = useQueryClient()
  const refresh = () =>
    Promise.all([
      invalidateWallets(queryClient),
      ...USAGE_QUERIES.map(schema =>
        queryClient.invalidateQueries({
          queryKey: createConnectQueryKey({ schema, cardinality: undefined }),
        })
      ),
    ])

  if (query.isLoading) return <Skeleton height={220} />
  if (query.isError) {
    return (
      <Card className="p-6 space-y-3" role="alert">
        <p className="text-sm text-destructive">The credit could not be loaded.</p>
        <Button variant="secondary" size="sm" onClick={() => void query.refetch()}>
          Retry
        </Button>
      </Card>
    )
  }
  const wallets = query.data?.wallets ?? []
  const wallet = wallets.find(w => w.id === selectedId) ?? wallets[0]
  const close = () => setAction(null)
  const live = wallet?.live
  const updatedAt =
    live?.asOf && !live.stale ? timestampDate(live.asOf) : new Date(query.dataUpdatedAt)

  return (
    <div className="space-y-4">
      {wallets.length > 0 && (
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex flex-wrap items-center gap-2">
            {wallets.length > 1 && (
              <Select value={wallet?.id} onValueChange={setSelectedId}>
                <SelectTrigger
                  className="h-8 w-auto min-w-[160px] gap-2 text-xs"
                  aria-label="Wallet"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {wallets.map(w => (
                    <SelectItem key={w.id} value={w.id}>
                      {walletTitle(w)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}
            {liveSubscriptionCount > 1 && (
              <Select
                value={scope ?? ALL_SUBSCRIPTIONS}
                onValueChange={value => setScopeId(value === ALL_SUBSCRIPTIONS ? undefined : value)}
              >
                <SelectTrigger
                  className="h-8 w-auto min-w-[180px] gap-2 text-xs"
                  aria-label="Subscription"
                >
                  <span className="text-muted-foreground">Subscription:</span>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value={ALL_SUBSCRIPTIONS}>All</SelectItem>
                  {liveSubscriptions.map(subscription => (
                    <SelectItem key={subscription.id} value={subscription.id}>
                      {subscriptionLabel(subscription.id)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            )}
          </div>
          <div className="flex items-center gap-2">
            <UpdatedStamp
              at={updatedAt}
              refreshing={query.isFetching}
              onRefresh={() => void refresh()}
            />
            {canManage && (
              <Button size="sm" variant="outline" onClick={() => setCreating('wallet')}>
                <Plus size={14} /> New wallet
              </Button>
            )}
          </div>
        </div>
      )}
      {!wallet ? (
        <Card className="overflow-hidden">
          <PanelEmptyState
            icon={WalletCards}
            title="No credit yet"
            description={
              canManage
                ? 'Add credit to start a balance.'
                : 'Credit notes, credit purchases and grants start a balance.'
            }
            action={
              canManage && (
                <Button size="sm" variant="outline" onClick={() => setCreating('add-credit')}>
                  <Plus size={14} /> Add credit
                </Button>
              )
            }
          />
        </Card>
      ) : (
        <WalletCard
          key={wallet.id}
          wallet={wallet}
          scope={scope}
          grantActivity={grantActivity}
          unbilledByGrant={unbilledByGrant}
          activity={activityQuery.data?.wallets.find(a => a.walletId === wallet.id)}
          activityState={
            activityQuery.isLoading ? 'loading' : activityQuery.isError ? 'error' : 'ready'
          }
          period={periods.get(wallet.id)}
          periodsStatus={periodsStatus}
          liveSubscriptionCount={liveSubscriptionCount}
          canManage={canManage}
          paymentMethods={paymentMethods}
          usageFilters={usageFilters}
          subscriptionLabel={subscriptionLabel}
          onAction={next => setAction({ action: next, wallet })}
          onRemove={grant => setRemoval({ wallet, grant })}
        />
      )}
      {wallet && <BalanceLagNote />}
      {wallets.length > 1 && <AllWalletsLedger customerId={customerId} />}
      {creating && (
        <CreateWalletDialog
          open
          onOpenChange={open => !open && setCreating(null)}
          customerId={customerId}
          currency={currency}
          onCreated={
            creating === 'add-credit'
              ? created => {
                  setCreating(null)
                  setSelectedId(created.id)
                  setAction({ action: 'add', wallet: created })
                }
              : undefined
          }
        />
      )}
      {action?.action === 'add' && (
        <AddCreditDialog
          open
          onOpenChange={close}
          wallet={action.wallet}
          customerCurrency={currency}
          paymentMethods={paymentMethods}
          defaultPaymentMethodId={defaultPaymentMethodId}
        />
      )}
      {action?.action === 'configure' && (
        <WalletSettingsDialog
          wallet={action.wallet}
          customerCurrency={currency}
          paymentMethods={paymentMethods}
          defaultPaymentMethodId={defaultPaymentMethodId}
          onClose={close}
        />
      )}
      {removal && (
        <RemoveCreditDialog
          open
          onOpenChange={() => setRemoval(null)}
          wallet={removal.wallet}
          grant={removal.grant}
        />
      )}
    </div>
  )
}

/** When the figures shown were computed, and a way to fetch them again. */
const UpdatedStamp = ({
  at,
  refreshing,
  onRefresh,
}: {
  at: Date
  refreshing: boolean
  onRefresh: () => void
}) => {
  const [, tick] = useState(0)
  useEffect(() => {
    const timer = setInterval(() => tick(n => n + 1), 30_000)
    return () => clearInterval(timer)
  }, [])
  return (
    <div className="flex items-center gap-1 text-xs text-muted-foreground">
      <span title={parseAndFormatDateTime(at.toISOString())}>
        Updated {formatDistanceToNow(at, { addSuffix: true })}
      </span>
      <Button
        size="icon"
        variant="ghost"
        className="h-7 w-7"
        aria-label="Refresh"
        disabled={refreshing}
        onClick={onRefresh}
      >
        <RefreshCw size={13} className={cn(refreshing && 'animate-spin')} />
      </Button>
    </div>
  )
}

const WalletCard = ({
  wallet,
  scope,
  grantActivity,
  unbilledByGrant,
  activity,
  activityState,
  period,
  periodsStatus,
  liveSubscriptionCount,
  canManage,
  paymentMethods,
  usageFilters,
  subscriptionLabel,
  onAction,
  onRemove,
}: {
  wallet: Wallet
  /** The subscription whose credit is shown: its own and the customer-wide credit. */
  scope?: string
  grantActivity: Map<string, GrantActivity>
  unbilledByGrant: Map<string, bigint>
  activity?: WalletActivity
  activityState: ActivityState
  period?: WalletPeriod
  periodsStatus: WalletPeriodsStatus
  liveSubscriptionCount: number
  canManage: boolean
  paymentMethods: CustomerPaymentMethod[]
  usageFilters: Map<string, string>
  subscriptionLabel: (subscriptionId: string) => string
  onAction: (action: Action) => void
  onRemove: (grant: CreditGrant) => void
}) => {
  const [tab, setTab] = useState<WalletTab>('activity')
  const title = walletTitle(wallet)
  // The title and the balance name the credit unit: amounts below them print bare.
  const local = useMemo(() => ({ ...wallet, bareUnit: true }), [wallet])
  const scoped = useMemo(() => {
    const inScope = (subscriptionId?: string) =>
      !scope || !subscriptionId || subscriptionId === scope
    return {
      ...local,
      grants: local.grants.filter(grant => inScope(grant.subscriptionId)),
      groups: local.groups.filter(group => inScope(group.subscriptionId)),
    }
  }, [local, scope])
  const funding = (period?.subscriptions ?? []).filter(s => !scope || s.id === scope)
  const live = slotItems('customerWallet.live')[0]
  const useScopedBalance = live?.useScopedBalance ?? noScopedBalance
  const scopeBalance = useScopedBalance(wallet, scope)
  // Without a live estimate, what the subscription can spend is its own and the customer-wide
  // credit on account.
  const spendable = live
    ? scopeBalance?.balance
    : scoped.groups.reduce((sum, group) => sum + group.available, 0n)
  const showGroups = scoped.groups.length > 0
  const showPeriod = !!wallet.live && !wallet.live.stale
  return (
    <Card className="space-y-5 p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <WalletBalanceSummary
          wallet={wallet}
          title={title}
          scoped={
            scope && spendable !== undefined
              ? { balance: spendable, subscription: subscriptionLabel(scope) }
              : undefined
          }
          notes={live?.notes(wallet)}
        />
        {canManage && (
          <div className="flex items-center gap-2">
            <Button size="sm" onClick={() => onAction('add')}>
              <Plus size={14} /> Add credit
            </Button>
            {hasWalletSettings(wallet) && (
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    size="icon"
                    variant="outline"
                    className="h-8 w-8"
                    aria-label={`${title} settings`}
                    onClick={() => onAction('configure')}
                  >
                    <Settings2 size={14} />
                  </Button>
                </TooltipTrigger>
                <TooltipContent>{title} settings</TooltipContent>
              </Tooltip>
            )}
          </div>
        )}
      </div>
      <div className="grid gap-3 empty:hidden lg:grid-cols-3">
        {showPeriod &&
          (periodsStatus === 'ready' ? (
            period &&
            funding.length > 0 && (
              <WalletPeriodUsage
                wallet={scoped}
                subscriptions={funding}
                scope={scope}
                subscriptionLabel={subscriptionLabel}
              />
            )
          ) : periodsStatus === 'loading' ? null : (
            <PeriodUsageUnavailable reason={periodsStatus} />
          ))}
        <Slot
          name="customerWallet.sections"
          wallet={local}
          paymentMethods={paymentMethods}
          canConfigure={canManage}
        />
      </div>
      {showGroups && (
        <div className="grid gap-3 sm:grid-cols-3">
          <CreditGroupsList
            groups={scoped.groups}
            denomination={local}
            usageFilters={usageFilters}
            unbilledByGrant={unbilledByGrant}
            activityState={activityState}
          />
        </div>
      )}
      {periodsStatus === 'ready' && funding.length > 0 && (
        <div className="border-t border-border pt-4">
          <WalletUsageChart wallet={local} scope={scope} />
        </div>
      )}
      <Tabs value={tab} onValueChange={value => setTab(value as WalletTab)}>
        <TabsList variant="ghost" aria-label={`${title} details`}>
          <TabsTrigger value="activity">Activity</TabsTrigger>
          <TabsTrigger value="grants">Grants</TabsTrigger>
          <TabsTrigger value="summary">Summary</TabsTrigger>
        </TabsList>
        <TabsContent value="activity" className="mt-3">
          <CustomerLedgerTable
            customerId={wallet.customerId}
            wallet={wallet}
            scope={scope ? { subscriptionId: scope, balance: scopeBalance } : undefined}
            limit={LEDGER_PREVIEW_ROWS}
          />
        </TabsContent>
        <TabsContent value="grants" className="mt-3">
          <CreditGrantsTable
            wallet={scoped}
            scope={scope}
            activity={grantActivity}
            activityState={activityState}
            liveSubscriptionCount={liveSubscriptionCount}
            canManage={canManage}
            onRemove={onRemove}
            usageFilters={usageFilters}
          />
        </TabsContent>
        <TabsContent value="summary" className="mt-3">
          <WalletSinceRow
            wallet={local}
            activity={activity}
            activityState={activityState}
            period={period && { ...period, subscriptions: funding }}
            periodsStatus={periodsStatus}
          />
        </TabsContent>
      </Tabs>
    </Card>
  )
}

/** Every wallet's movements in one statement, folded away: each wallet's card has its own. */
const AllWalletsLedger = ({ customerId }: { customerId: string }) => {
  const [open, setOpen] = useState(false)
  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <Card className="p-5">
        <CollapsibleTrigger className="group flex items-center gap-1 text-sm font-medium">
          <ChevronDown className="h-4 w-4 transition-transform group-data-[state=open]:rotate-180" />
          All credit activity
        </CollapsibleTrigger>
        <CollapsibleContent className="pt-4">
          <CustomerLedgerTable customerId={customerId} limit={LEDGER_PREVIEW_ROWS} />
        </CollapsibleContent>
      </Card>
    </Collapsible>
  )
}

/** Why the wallet's usage this period is not shown: a partial sum would read as the total. */
const PeriodUsageUnavailable = ({ reason }: { reason: 'too-many' | 'error' }) => (
  <p className="text-xs text-muted-foreground lg:col-span-2">
    {reason === 'too-many'
      ? `Usage this period is not shown for customers with more than ${MAX_FUNDING_SUBSCRIPTIONS} live subscriptions.`
      : 'Usage this period could not be loaded. Reload the page to try again.'}
  </p>
)

/** The usage so far this period of every subscription drawing on the wallet (or of `scope`), as
 * the server prices it for the chart below, and its projection as the server's outlooks state it.
 * Measured against the credit granted this period when the plan grants some. */
const WalletPeriodUsage = ({
  wallet,
  subscriptions,
  scope,
  subscriptionLabel,
}: {
  wallet: Wallet & { bareUnit?: boolean }
  subscriptions: FundingSubscription[]
  scope?: string
  subscriptionLabel: (subscriptionId: string) => string
}) => {
  const subscriptionIds = subscriptions.map(subscription => subscription.id)
  const usage = useQuery(listWalletDailyUsageCost, walletDailyUsageInput(wallet, scope, 'current'))
  const useProjection = slotItems('customerWallet.live')[0]?.useProjection ?? noProjection
  const projection = useProjection({ wallet, subscriptionIds, subscriptionLabel })
  if (usage.isError) return <PeriodUsageUnavailable reason="error" />
  if (!usage.data) return null
  const used = dailyUsageTotal(usage.data.rows)
  const since = subscriptions.map(subscription => subscription.start).sort()[0]
  const credit = periodCreditGranted(wallet.grants, since, new Date(), scope)
  const utilization = periodUtilization(used, credit)
  // An idle wallet shows no bar: an empty one reads as noise.
  if (used === 0n && !projection?.projected && !utilization) return null
  const end = subscriptions[0].end
  return (
    <div className="rounded-lg border border-border p-4 lg:col-span-2">
      <PeriodUsageBar
        available={
          scope ? wallet.groups.reduce((sum, group) => sum + group.available, 0n) : wallet.available
        }
        used={used}
        projection={projection}
        denomination={wallet}
        resetsOn={subscriptions.every(subscription => subscription.end === end) ? end : undefined}
        note={wallet.creditUnit ? undefined : 'excl. tax'}
        utilization={utilization && { ...utilization, added: credit.added }}
      />
    </div>
  )
}
