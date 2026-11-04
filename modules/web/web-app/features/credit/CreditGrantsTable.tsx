import {
  Badge,
  Button,
  Skeleton,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@md/ui'
import { cn } from '@ui/lib'
import { ChevronRight } from 'lucide-react'
import { Fragment, useMemo, useState } from 'react'
import { Link } from 'react-router-dom'

import Pagination from '@/components/table/CustomTable/components/Pagination'
import { ProductScopeBadge } from '@/features/credit/ProductScopeBadge'
import { formatWalletAmount } from '@/features/credit/format'
import { formatDate, grantSourceLabel } from '@/features/credit/labels'
import {
  USED_UP_PAGE_SIZE,
  grantUsed,
  isDrained,
  isUsedUp,
  usedUpToggleLabel,
} from '@/features/credit/usedUpGrants'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { formatCurrency } from '@/lib/utils/numbers'
import {
  CreditAppliesTo,
  CreditGrantKind,
  CreditGrantSource,
  CreditGrantStatus,
  CreditGrantType,
} from '@/rpc/api/wallets/v1/models_pb'
import { listWalletGrants } from '@/rpc/api/wallets/v1/wallets-WalletsService_connectquery'

import type { CreditGrant, GrantActivity, Wallet } from '@/rpc/api/wallets/v1/models_pb'

export type ActivityState = 'loading' | 'error' | 'ready'

// Mirrors the server's default draw priority: only a grant moved ahead of or behind it says so.
const DEFAULT_PRIORITY = 100

const grantName = (grant: CreditGrant, activity?: GrantActivity) => activity?.name || grant.reason

/**
 * The wallet's grants in the order invoices spend them; used-up grants fold away and load page by
 * page, newest first.
 */
export const CreditGrantsTable = ({
  wallet,
  scope,
  activity,
  activityState,
  liveSubscriptionCount,
  canManage,
  onRemove,
  usageFilters,
}: {
  wallet: Wallet
  /** The subscription whose credit is shown: its own and the customer-wide credit. */
  scope?: string
  activity: Map<string, GrantActivity>
  activityState: ActivityState
  /** "Any subscription" is worth saying only once the customer has several. */
  liveSubscriptionCount: number
  canManage: boolean
  onRemove: (grant: CreditGrant) => void
  /** The usage filter of each live subscription that has one, e.g. "team = mobile". */
  usageFilters: Map<string, string>
}) => {
  const [showUsedUp, setShowUsedUp] = useState(false)
  // A page of another scope's used-up grants starts over.
  const [paging, setPaging] = useState({ scope, page: 0 })
  const page = paging.scope === scope ? paging.page : 0
  const usedUp = useQuery(listWalletGrants, {
    customerId: wallet.customerId,
    walletId: wallet.id,
    subscriptionId: scope,
    usedUp: true,
    pagination: { page, perPage: USED_UP_PAGE_SIZE },
  })
  const usedUpActivity = useMemo(
    () => new Map((usedUp.data?.activity ?? []).map(a => [a.creditGrantId, a] as const)),
    [usedUp.data]
  )
  const current = wallet.grants.filter(grant => !isUsedUp(grant))
  const live = current.filter(grant => !isDrained(grant, activity.get(grant.id)))
  const drained = current.filter(grant => isDrained(grant, activity.get(grant.id)))
  const storedUsedUp = usedUp.data?.paginationMeta?.totalItems ?? 0
  const usedUpCount = storedUsedUp + drained.length
  if (live.length === 0 && usedUp.isLoading) return <Skeleton height={60} />
  if (live.length === 0 && usedUpCount === 0 && !usedUp.isError) {
    return <p className="text-xs text-muted-foreground">No grants yet.</p>
  }

  const row = (grant: CreditGrant, grantActivity?: GrantActivity, state = activityState) => (
    <GrantRow
      key={grant.id}
      grant={grant}
      wallet={wallet}
      activity={grantActivity}
      activityState={state}
      severalSubscriptions={liveSubscriptionCount >= 2}
      usageFilter={grant.subscriptionId ? usageFilters.get(grant.subscriptionId) : undefined}
      canRemove={canManage && grant.removable}
      onRemove={() => onRemove(grant)}
    />
  )

  return (
    <div className="space-y-2">
      <p className="text-xs text-muted-foreground">Credit is used in this order.</p>
      {activityState === 'error' && (
        <p className="text-xs text-destructive" role="alert">
          How the grants were consumed could not be loaded.
        </p>
      )}
      <div className="overflow-x-auto">
        <Table className="min-w-[560px] text-xs">
          <TableHeader>
            <TableRow>
              <TableHead className="text-xs">Grant</TableHead>
              <TableHead className="w-[28%] text-xs">Used</TableHead>
              <TableHead className="text-right text-xs">Left</TableHead>
              <TableHead className="text-right text-xs">Expires</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {live.map(grant => row(grant, activity.get(grant.id)))}
            {showUsedUp && page === 0 && drained.map(grant => row(grant, activity.get(grant.id)))}
            {showUsedUp &&
              usedUp.data?.grants.map(grant => row(grant, usedUpActivity.get(grant.id), 'ready'))}
          </TableBody>
        </Table>
      </div>
      {usedUp.isError && (
        <p className="text-xs text-destructive" role="alert">
          The used-up grants could not be loaded.
        </p>
      )}
      {usedUpCount > 0 && (
        <div className="flex flex-wrap items-center justify-between gap-2">
          <Button
            variant="link"
            size="sm"
            className="h-auto p-0 text-xs"
            onClick={() => setShowUsedUp(!showUsedUp)}
          >
            {usedUpToggleLabel(usedUpCount, showUsedUp)}
          </Button>
          {showUsedUp && storedUsedUp > USED_UP_PAGE_SIZE && (
            <Pagination
              variant="footer"
              pagination={{ pageIndex: page, pageSize: USED_UP_PAGE_SIZE }}
              setPagination={next => {
                const value =
                  typeof next === 'function'
                    ? next({ pageIndex: page, pageSize: USED_UP_PAGE_SIZE })
                    : next
                setPaging({ scope, page: value.pageIndex })
              }}
              totalCount={storedUsedUp}
              isLoading={usedUp.isFetching}
            />
          )}
        </div>
      )}
    </div>
  )
}

const GrantRow = ({
  grant,
  wallet,
  activity,
  activityState,
  severalSubscriptions,
  usageFilter,
  canRemove,
  onRemove,
}: {
  grant: CreditGrant
  wallet: Wallet
  activity?: GrantActivity
  activityState: ActivityState
  severalSubscriptions: boolean
  usageFilter?: string
  canRemove: boolean
  onRemove: () => void
}) => {
  const [open, setOpen] = useState(false)
  const amount = (minor: bigint) => formatWalletAmount(minor, wallet)
  const expired = grant.status === CreditGrantStatus.EXPIRED
  const pending = grant.status === CreditGrantStatus.PENDING
  const invoiced = activity?.invoiced ?? 0n
  const atExpiry = activity?.settledAtExpiry ?? 0n
  const unbilled = activity?.unbilled ?? 0n
  const removed = activity?.removed ?? 0n
  const rolledOver = activity?.rolledOver ?? 0n
  const left = grant.remaining - unbilled
  const name = grantName(grant, activity)
  const untouched =
    invoiced === 0n && atExpiry === 0n && unbilled === 0n && removed === 0n && rolledOver === 0n
  const consumed = untouched
    ? pending
      ? `Usable from ${formatDate(grant.effectiveAt)}`
      : 'Not used yet'
    : [
        invoiced !== 0n && `${amount(invoiced)} invoiced`,
        atExpiry !== 0n && `${amount(atExpiry)} applied at expiry`,
        unbilled !== 0n && `${amount(unbilled)} unbilled`,
        removed !== 0n && `${amount(removed)} removed`,
        rolledOver !== 0n && `${amount(rolledOver)} rolled over`,
      ]
        .filter(Boolean)
        .join(' · ')
  const detailsId = `grant-${grant.id}-details`

  return (
    <Fragment>
      <TableRow
        className={cn(
          (isUsedUp(grant) || isDrained(grant, activity)) && 'text-muted-foreground',
          open && 'border-b-0'
        )}
      >
        <TableCell className="align-top">
          <div className="flex items-start gap-1">
            <button
              type="button"
              aria-expanded={open}
              aria-controls={detailsId}
              aria-label={`${open ? 'Hide' : 'Show'} details of ${name}`}
              onClick={() => setOpen(!open)}
              className="-ml-1 rounded p-0.5 text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              <ChevronRight
                className={cn('h-3.5 w-3.5 transition-transform', open && 'rotate-90')}
              />
            </button>
            <div className="min-w-0">
              <span className="font-medium text-foreground">{name}</span>
              <GrantSource grant={grant} activity={activity} />
              {canRemove && (
                <div>
                  <Button
                    variant="link"
                    size="sm"
                    className="h-auto p-0 text-xs"
                    onClick={onRemove}
                  >
                    Remove
                  </Button>
                </div>
              )}
            </div>
          </div>
        </TableCell>
        <TableCell className="align-top">
          {activityState === 'loading' ? (
            <Skeleton height={8} />
          ) : activityState === 'error' ? (
            <span className="text-muted-foreground">—</span>
          ) : pending && untouched ? (
            <span className="text-muted-foreground">{consumed}</span>
          ) : (
            <>
              <span className="tabular-nums">{amount(grantUsed(activity))}</span>
              <ConsumedBar
                total={grant.amount}
                invoiced={invoiced}
                atExpiry={atExpiry}
                unbilled={unbilled}
                label={`${consumed}, of ${amount(grant.amount)}`}
              />
            </>
          )}
        </TableCell>
        <TableCell className="align-top text-right tabular-nums">
          {expired ? (
            `${amount(grant.expiredAmount ?? 0n)} expired`
          ) : activityState === 'ready' ? (
            <span className="font-medium text-foreground">{amount(left)}</span>
          ) : activityState === 'loading' ? (
            <Skeleton height={14} />
          ) : (
            <span className="text-muted-foreground">—</span>
          )}
        </TableCell>
        <TableCell className="align-top whitespace-nowrap text-right">
          {grant.endedAt
            ? `Ended ${formatDate(grant.endedAt)}`
            : grant.expiresAt
              ? formatDate(grant.expiresAt)
              : 'Never'}
        </TableCell>
      </TableRow>
      {open && (
        <TableRow id={detailsId} className="bg-muted/30 hover:bg-muted/30">
          <TableCell colSpan={4} className="space-y-2 pl-7">
            <p className="text-muted-foreground tabular-nums">
              {[
                `Added ${formatDate(grant.effectiveAt)}`,
                `${amount(grant.amount)} granted`,
                activityState === 'ready' && consumed,
                grant.endedAt && 'Ended with its subscription',
              ]
                .filter(Boolean)
                .join(' · ')}
            </p>
            <GrantChips
              grant={grant}
              activity={activity}
              severalSubscriptions={severalSubscriptions}
              usageFilter={usageFilter}
            />
          </TableCell>
        </TableRow>
      )}
    </Fragment>
  )
}

/** Where the grant came from, in one link or label: its plan, its invoice, else its source. */
const GrantSource = ({ grant, activity }: { grant: CreditGrant; activity?: GrantActivity }) => {
  const basePath = useBasePath()
  const source = grant.subscriptionId ? (
    <Link className="hover:underline" to={`${basePath}/subscriptions/${grant.subscriptionId}`}>
      {grant.subscriptionPlanName ?? grant.kindLabel}
    </Link>
  ) : grant.invoiceId ? (
    <Link className="hover:underline" to={`${basePath}/invoices/${grant.invoiceId}`}>
      {activity?.invoiceNumber ?? 'Invoice'}
    </Link>
  ) : (
    grantSourceLabel[grant.source]
  )
  return <span className="text-muted-foreground"> · {source}</span>
}

const GrantChips = ({
  grant,
  activity,
  severalSubscriptions,
  usageFilter,
}: {
  grant: CreditGrant
  activity?: GrantActivity
  severalSubscriptions: boolean
  usageFilter?: string
}) => {
  const basePath = useBasePath()
  // Included credit names its plan in its label already: that label links to the subscription.
  const included = grant.kind === CreditGrantKind.INCLUDED
  const subscription = grant.subscriptionId && (
    <Link to={`${basePath}/subscriptions/${grant.subscriptionId}`}>
      <Badge size="sm" variant={included ? 'secondary' : 'outline'}>
        {included ? grant.kindLabel : `For ${grant.subscriptionPlanName ?? 'its subscription'}`}
      </Badge>
    </Link>
  )
  const invoice = grant.invoiceId && (
    <Link className="underline" to={`${basePath}/invoices/${grant.invoiceId}`}>
      {activity?.invoiceNumber ?? 'Invoice'}
    </Link>
  )
  const chips: { label: string; variant: 'secondary' | 'outline' }[] = []
  if (activity?.automaticTopUp) chips.push({ label: 'Automatic top-up', variant: 'outline' })
  if (grant.kindLabel && !(included && subscription))
    chips.push({ label: grant.kindLabel, variant: 'secondary' })
  if (grant.source === CreditGrantSource.ROLLOVER)
    chips.push({ label: 'Rolled over', variant: 'outline' })
  // Rollovers are minted one step ahead of their source grant: an engine detail, not an operator's.
  if (grant.priority !== DEFAULT_PRIORITY && grant.source !== CreditGrantSource.ROLLOVER)
    chips.push({ label: `Priority ${grant.priority}`, variant: 'outline' })

  const paid = grant.type === CreditGrantType.PAID
  return (
    <div className="flex flex-wrap items-center gap-1.5 text-muted-foreground">
      {paid && (
        <Badge size="sm" variant="outline">
          Paid
        </Badge>
      )}
      {paid && grant.cost !== undefined && grant.costCurrency && (
        <span className="tabular-nums">{formatCurrency(grant.cost, grant.costCurrency)}</span>
      )}
      {paid &&
        !grant.invoiceId &&
        (grant.source === CreditGrantSource.MANUAL || grant.source === CreditGrantSource.API) && (
          <span>Paid outside Meteroid</span>
        )}
      {chips.map(chip => (
        <Badge key={chip.label} size="sm" variant={chip.variant}>
          {chip.label}
        </Badge>
      ))}
      {subscription}
      {subscription && usageFilter && <span className="text-xs">{usageFilter}</span>}
      {!grant.subscriptionId && severalSubscriptions && (
        <Badge size="sm" variant="outline">
          Any subscription
        </Badge>
      )}
      {grant.appliesTo === CreditAppliesTo.USAGE && (
        <Badge size="sm" variant="outline">
          Usage only
        </Badge>
      )}
      <ProductScopeBadge products={grant.applicableProducts} />
      {invoice && (
        <>
          <span aria-hidden>·</span>
          {invoice}
        </>
      )}
      {grant.reference && <span>· {grant.reference}</span>}
    </div>
  )
}

/** Invoiced usage solid, usage paid at expiry lighter, unbilled usage lightest, of the grant. */
const ConsumedBar = ({
  total,
  invoiced,
  atExpiry,
  unbilled,
  label,
}: {
  total: bigint
  invoiced: bigint
  atExpiry: bigint
  unbilled: bigint
  label: string
}) => {
  const pct = (part: bigint) =>
    total > 0n ? Math.min(100, Math.max(0, (Number(part) / Number(total)) * 100)) : 0
  const invoicedPct = pct(invoiced)
  const atExpiryPct = Math.min(100 - invoicedPct, pct(atExpiry))
  const unbilledPct = Math.min(100 - invoicedPct - atExpiryPct, pct(unbilled))
  return (
    <div
      role="img"
      aria-label={label}
      title={label}
      className="mt-1 flex h-1.5 w-full min-w-24 overflow-hidden rounded-full bg-muted"
    >
      <div className="h-full bg-brand" style={{ width: `${invoicedPct}%` }} />
      <div className="h-full bg-brand/70" style={{ width: `${atExpiryPct}%` }} />
      <div className="h-full bg-brand/30" style={{ width: `${unbilledPct}%` }} />
    </div>
  )
}
