import { timestampDate } from '@bufbuild/protobuf/wkt'
import { createQueryOptions, useTransport } from '@connectrpc/connect-query'
import { Skeleton } from '@md/ui'
import { useQueries } from '@tanstack/react-query'

import { formatWalletAmount } from '@/features/credit/format'
import { formatDate, grantSourceLabel } from '@/features/credit/labels'
import { MAX_FUNDING_SUBSCRIPTIONS } from '@/features/credit/useWalletPeriods'
import { walletAverageUsage, walletMonthlyIncluded } from '@/features/credit/walletPeriodUsage'
import { plural } from '@/features/subscriptions/subscriptionOverview'
import { toMinor } from '@/features/subscriptions/subscriptionPrice'
import { isUsageLine, periodUsageFor } from '@/features/subscriptions/subscriptionWallets'
import { currencyPrecision } from '@/lib/data/currency-precision'
import { formatDateOnly } from '@/lib/utils/date'
import {
  getInvoice,
  listInvoices,
} from '@/rpc/api/invoices/v1/invoices-InvoicesService_connectquery'
import { InvoiceStatus } from '@/rpc/api/invoices/v1/models_pb'
import { getSubscriptionDetails } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { CreditGrantType } from '@/rpc/api/wallets/v1/models_pb'

import type { WalletPeriod, WalletPeriodsStatus } from '@/features/credit/useWalletPeriods'
import type { CreditAddedGroup, Wallet, WalletActivity } from '@/rpc/api/wallets/v1/models_pb'

// The average covers this many complete periods: recent enough to show a trend.
const AVERAGE_PERIODS = 3
// Enough rows to find three billed periods past consolidated parents and one-off invoices.
const RECENT_INVOICES = 8
const DAY_MS = 24 * 60 * 60 * 1000

const addedSummary = (groups: CreditAddedGroup[], amount: (minor: bigint) => string) => {
  const parts = groups
    .filter(g => g.amount !== 0n)
    .map(g => {
      const name = g.name || grantSourceLabel[g.source]
      return g.grants > 1 ? `${g.grants} × ${name}` : `${amount(g.amount)} ${name}`
    })
  if (parts.length === 0) return 'No credit added'
  return parts.length > 3
    ? `${parts.slice(0, 3).join(' + ')} + ${parts.length - 3} more`
    : parts.join(' + ')
}

/**
 * Whether the credit used counts tax: paid credit pays invoices tax included, promotional credit
 * comes off before tax. Credits of a unit carry no tax of their own.
 */
export const usedTaxBasis = (
  wallet: Pick<Wallet, 'creditUnit' | 'grantTypes'>
): 'incl' | 'excl' | 'mixed' | undefined => {
  if (wallet.creditUnit) return undefined
  const paid = wallet.grantTypes.includes(CreditGrantType.PAID)
  const promotional = wallet.grantTypes.includes(CreditGrantType.PROMOTIONAL)
  if (paid && promotional) return 'mixed'
  if (paid) return 'incl'
  return promotional ? 'excl' : undefined
}

const USED_LABEL = { incl: 'Used · incl. tax', excl: 'Used · excl. tax', mixed: 'Used' } as const

/** The wallet's first credit, else its creation. */
const walletSince = (wallet: Wallet) => {
  const since = wallet.firstCreditAt ?? wallet.createdAt
  return since ? timestampDate(since) : undefined
}

const Tile = ({ label, value, sub }: { label: string; value: string; sub?: string }) => (
  <div className="flex min-w-0 flex-col gap-0.5">
    <span className="text-xs text-muted-foreground">{label}</span>
    <span className="font-semibold tabular-nums text-foreground">{value}</span>
    {sub && <span className="text-xs text-muted-foreground">{sub}</span>}
  </div>
)

/**
 * Usage of each funding subscription's latest complete periods, as its issued invoices billed it.
 * A draft merged into a consolidated invoice holds the subscription's own lines, so it counts.
 */
const useRecentUsage = (wallet: Wallet, period?: WalletPeriod) => {
  const transport = useTransport()
  const subscriptions = period?.subscriptions ?? []
  const lists = useQueries({
    queries: subscriptions.map(subscription =>
      createQueryOptions(
        listInvoices,
        {
          subscriptionId: subscription.id,
          orderBy: 'billing_period_start.desc',
          pagination: { perPage: RECENT_INVOICES, page: 0 },
        },
        { transport }
      )
    ),
  })
  const billed = lists.flatMap((list, i) =>
    (list.data?.invoices ?? [])
      .filter(
        invoice =>
          invoice.subscriptionId &&
          (invoice.status === InvoiceStatus.FINALIZED || !!invoice.consolidatedIntoInvoiceId) &&
          invoice.billingPeriodStart &&
          new Date(invoice.billingPeriodStart) <= new Date(subscriptions[i].start)
      )
      .map(invoice => ({ invoice, subscriptionId: subscriptions[i].id }))
  )
  const details = useQueries({
    queries: billed.map(({ invoice }) =>
      createQueryOptions(getInvoice, { id: invoice.id }, { transport })
    ),
  })
  const loading = lists.some(l => l.isLoading) || details.some(d => d.isLoading)
  const error = lists.some(l => l.isError) || details.some(d => d.isError)
  const perSubscription = subscriptions.map(subscription =>
    billed
      .flatMap(({ invoice: row, subscriptionId }, i) => {
        const invoice = details[i]?.data?.invoice
        return subscriptionId === subscription.id && invoice && invoice.lineItems.some(isUsageLine)
          ? [{ period: row.billingPeriodStart ?? '', usage: periodUsageFor(wallet, invoice) }]
          : []
      })
      .sort((a, b) => b.period.localeCompare(a.period))
      .slice(0, AVERAGE_PERIODS)
      .map(u => u.usage)
  )
  return { loading, error, perSubscription }
}

/** Lifetime totals of a wallet and its recent average usage, for the account manager. */
export const WalletSinceRow = ({
  wallet,
  activity,
  activityState,
  period,
  periodsStatus,
}: {
  wallet: Wallet
  activity?: WalletActivity
  activityState: 'loading' | 'error' | 'ready'
  period?: WalletPeriod
  periodsStatus: WalletPeriodsStatus
}) => {
  const transport = useTransport()
  const recent = useRecentUsage(wallet, period)
  const details = useQueries({
    queries: (period?.subscriptions ?? []).map(subscription =>
      createQueryOptions(getSubscriptionDetails, { subscriptionId: subscription.id }, { transport })
    ),
  })
  const since = walletSince(wallet)
  const grid = 'grid grid-cols-2 gap-4 md:grid-cols-4'
  const heading = (
    <h4 className="text-sm font-medium text-foreground">
      {since ? `Since ${formatDateOnly(since.toISOString())}` : 'Since the first credit'}
    </h4>
  )

  if (activityState === 'loading') {
    return (
      <div className="space-y-3">
        {heading}
        <div className={grid}>
          {[0, 1, 2, 3].map(i => (
            <Skeleton key={i} height={44} />
          ))}
        </div>
      </div>
    )
  }
  if (activityState === 'error' || !activity) {
    return (
      <div className="space-y-3">
        {heading}
        <p className="text-xs text-muted-foreground">The credit history could not be loaded.</p>
      </div>
    )
  }

  const amount = (minor: bigint) => formatWalletAmount(minor, wallet)
  const unbilled = wallet.live && !wallet.live.stale ? wallet.live.pendingUsage : 0n
  const used = activity.invoiced + activity.settledAtExpiry + unbilled
  const taxBasis = usedTaxBasis(wallet)
  const expirySub = [
    activity.removed > 0n && `${amount(activity.removed)} removed`,
    activity.nextExpiry && `next expiry ${formatDate(activity.nextExpiry)}`,
  ].filter((s): s is string => Boolean(s))

  const average = walletAverageUsage(recent.perSubscription)
  const monthly =
    !!period &&
    period.subscriptions.every(
      s => Math.abs(new Date(s.end).getTime() - new Date(s.start).getTime()) / DAY_MS <= 31
    )
  const count = average?.periods ?? 0
  const averageLabel = monthly
    ? `Avg usage / month · last ${plural(count, 'month')}`
    : `Avg usage / period · last ${plural(count, 'period')}`
  const funding = details.every(d => d.data?.subscription)
    ? details.map(d => ({
        currency: d.data!.subscription!.currency,
        fees: [
          ...d.data!.priceComponents.map(c => ({ fee: c.fee, period: c.period })),
          ...d.data!.addOns.map(a => ({ fee: a.fee, period: a.period, quantity: a.quantity })),
        ],
      }))
    : undefined
  const included =
    funding && funding.length > 0 && monthly
      ? walletMonthlyIncluded(funding, wallet.creditUnit?.id ?? wallet.currency)
      : undefined
  // A partial set of subscriptions would read as the wallet's average.
  const averageUnknown = periodsStatus === 'too-many' || periodsStatus === 'error' || recent.error
  const precision = wallet.creditUnit?.precision ?? currencyPrecision(wallet.currency)

  return (
    <div className="space-y-3">
      {heading}
      <div className={grid}>
        <Tile
          label="Added"
          value={amount(activity.creditAdded)}
          sub={addedSummary(activity.creditAddedGroups, amount)}
        />
        <Tile
          label={taxBasis ? USED_LABEL[taxBasis] : 'Used'}
          value={amount(used)}
          sub={`${[
            `${amount(activity.invoiced)} invoiced`,
            activity.settledAtExpiry !== 0n && `${amount(activity.settledAtExpiry)} at expiry`,
            `${amount(unbilled)} unbilled`,
          ]
            .filter(Boolean)
            .join(' + ')}${taxBasis === 'mixed' ? ' · paid credit incl. tax' : ''}`}
        />
        <Tile
          label="Expired"
          value={amount(activity.expired)}
          sub={expirySub.length > 0 ? expirySub.join(' · ') : undefined}
        />
        {averageUnknown ? (
          <Tile
            label="Avg usage / month"
            value="—"
            sub={
              periodsStatus === 'too-many'
                ? `Not shown past ${MAX_FUNDING_SUBSCRIPTIONS} live subscriptions`
                : 'Could not be loaded'
            }
          />
        ) : periodsStatus === 'loading' || recent.loading ? (
          <Skeleton height={44} />
        ) : average !== undefined ? (
          <Tile
            label={averageLabel}
            value={amount(average.average)}
            sub={included ? `vs ${amount(toMinor(included, precision))} included` : undefined}
          />
        ) : (
          <Tile label="Avg usage / month" value="—" sub="No complete period invoiced yet" />
        )}
      </div>
    </div>
  )
}
