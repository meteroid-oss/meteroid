import { skipToken } from '@connectrpc/connect-query'
import { Skeleton, cn } from '@md/ui'
import { ReactNode } from 'react'
import { Link } from 'react-router'

import { creditUnitName, formatWalletAmount, walletTitle } from '@/features/credit/format'
import { paymentMethodLabel } from '@/features/credit/paymentMethods'
import {
  DashboardCard,
  DashboardCardTitle,
  dashedLink,
} from '@/features/dashboard/cards/DashboardCard'
import { formatShortDay } from '@/features/subscriptions/utils/dates'
import { useBasePath } from '@/hooks/useBasePath'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { usePermissions } from '@/hooks/usePermissions'
import { useQuery } from '@/lib/connectrpc'
import { formatMoney } from '@/lib/utils/format'
import { getCustomerSummary } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { SubscriptionStatus } from '@/rpc/api/subscriptions/v1/models_pb'
import {
  getUpcomingInvoice,
  listSubscriptions,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { listCustomerWallets } from '@/rpc/api/wallets/v1/wallets-WalletsService_connectquery'

import type { CustomerMoneyTotal } from '@/rpc/api/customers/v1/customers_pb'
import type { Customer } from '@/rpc/api/customers/v1/models_pb'
import type { UpcomingInvoice } from '@/rpc/api/subscriptions/v1/subscriptions_pb'
import type { Wallet } from '@/rpc/api/wallets/v1/models_pb'

const SUMMARY_STALE_MS = 60_000

export const useCustomerSummary = (customerId: string) =>
  useQuery(getCustomerSummary, { customerId }, { staleTime: SUMMARY_STALE_MS })

const sumText = (totals: CustomerMoneyTotal[]) =>
  totals.map(t => formatMoney(Number(t.amountCents), t.currency)).join(' + ')

// An upcoming invoice is computed on request and may fail where usage can't be read: one retry
// rather than the default three, so the tile settles in a second or two instead of pulsing.
const PREVIEW_RETRIES = 1

/** The value and its line below, while they load: still bars where they will appear. */
const ValuePlaceholder = () => (
  <div className="flex flex-col gap-2" aria-hidden>
    <Skeleton width="55%" height={21} className="rounded-[4px]" />
    <Skeleton width="35%" height={12} className="rounded-[4px]" />
  </div>
)

/** A summary tile, styled as the Home page's stat cards. */
const Tile = ({
  label,
  value,
  meta,
  href,
  loading,
}: {
  label: ReactNode
  value?: ReactNode
  meta?: ReactNode
  href?: string
  /** Keeps the tile's frame and title, with placeholders for the value. */
  loading?: boolean
}) => (
  // A fixed height: the value and up to two lines below it always fit, so a tile never grows
  // when its data arrives.
  // Home's tile layout, raised as the subscriptions list: in dark a fill a step lighter than the
  // surface (important: the themes set Home's tiles to --card), framed and rounded as the tables.
  <DashboardCard className="flex h-[125px] min-w-0 flex-col justify-between gap-2 rounded-lg border border-border bg-card dark:!bg-foreground/[0.035]">
    <DashboardCardTitle className="min-w-0 truncate leading-5">
      {href ? (
        // Where the tile leads: its title, with a fine dashed underline.
        <Link to={href} className={cn('hover:text-foreground', dashedLink)}>
          {label}
        </Link>
      ) : (
        label
      )}
    </DashboardCardTitle>
    {loading ? (
      <ValuePlaceholder />
    ) : (
      <div className="flex min-w-0 flex-col gap-1.5">
        <div className="truncate font-display text-[21px] leading-none tabular-nums text-foreground">
          {value}
        </div>
        {meta && <div className="line-clamp-2 text-xs leading-4 text-muted-foreground">{meta}</div>}
      </div>
    )}
  </DashboardCard>
)

// As many columns as tiles on a wide screen, so the row is always filled.
const WIDE_COLUMNS: Record<number, string> = {
  1: 'xl:grid-cols-1',
  2: 'xl:grid-cols-2',
  3: 'xl:grid-cols-3',
  4: 'xl:grid-cols-4',
}

/**
 * What credit the account holds, what it is worth, what it is charged next and what it owes. A
 * tile the caller may not see is left out. Which tiles show depends on permissions alone, never
 * on the data, and each loads in its own frame: nothing moves while the strip fills in.
 */
export const CustomerSummaryStrip = ({
  customerId,
  customer,
}: {
  customerId: string
  customer?: Pick<Customer, 'currency' | 'paymentMethods' | 'currentPaymentMethodId'>
}) => {
  const permissions = usePermissions()
  const isExpress = useIsExpressOrganization()
  const canViewSubscriptions = permissions.has('billing:subscriptions:read')
  const canViewCredit = permissions.has('billing:invoices:read') && !isExpress
  // The summary carries the receivables exactly when the caller may read invoices.
  const canViewReceivables = permissions.has('billing:invoices:read')
  const summary = useCustomerSummary(customerId)
  const data = summary.data
  const loading = summary.isLoading

  const receivables = data?.receivables
  const overdue = receivables ? sumText(receivables.overdue) : ''
  const converted = !!customer && !!data && customer.currency !== data.reportingCurrency
  const defaultMethod = customer?.paymentMethods.find(
    method => method.id === customer.currentPaymentMethodId
  )
  const count =
    1 + Number(canViewCredit) + Number(canViewSubscriptions) + Number(canViewReceivables)

  return (
    <div className={cn('grid grid-cols-2 gap-3', WIDE_COLUMNS[count])}>
      {canViewCredit && <CreditBalanceTile customerId={customerId} currency={customer?.currency} />}
      <Tile
        label={converted ? `Recurring · ${data.reportingCurrency}` : 'Recurring'}
        loading={loading}
        value={
          data ? (
            <>
              {formatMoney(Number(data.mrrCents), data.reportingCurrency)}
              <span className="font-sans text-xs text-muted-foreground"> / month</span>
            </>
          ) : (
            '—'
          )
        }
        meta={
          !data
            ? undefined
            : converted
              ? `Converted to ${data.reportingCurrency}, the reporting currency`
              : (data.term?.planName ?? 'Active subscriptions')
        }
      />
      {canViewSubscriptions && (
        <NextChargeTile
          customerId={customerId}
          paymentMethod={defaultMethod ? paymentMethodLabel(defaultMethod) : undefined}
        />
      )}
      {canViewReceivables && (
        <Tile
          label="Outstanding"
          loading={loading}
          value={
            !receivables
              ? '—'
              : receivables.open.length > 0
                ? sumText(receivables.open)
                : 'Nothing due'
          }
          meta={
            overdue ? (
              <span className="flex items-center gap-1.5 text-tag-destructive">
                <span aria-hidden className="size-1.5 shrink-0 rounded-full bg-current" />
                {overdue} overdue
              </span>
            ) : receivables ? (
              'Nothing overdue'
            ) : undefined
          }
        />
      )}
    </div>
  )
}

/** The plan's credit first (a credit-unit wallet), else the account credit in the customer's currency. */
const CreditBalanceTile = ({ customerId, currency }: { customerId: string; currency?: string }) => {
  const query = useQuery(listCustomerWallets, { customerId }, { staleTime: SUMMARY_STALE_MS })
  const wallets = query.data?.wallets ?? []
  if (query.isLoading) return <Tile label="Credit balance" loading />
  if (wallets.length === 0)
    return <Tile label="Credit balance" value="—" meta="No credit" href="?tab=wallets" />
  const primary =
    wallets.find(wallet => wallet.creditUnit) ??
    wallets.find(wallet => wallet.currency === currency) ??
    wallets[0]
  const available = (wallet: Wallet) => wallet.live?.availableAfterUsage ?? wallet.available
  const balance = (wallet: Wallet) =>
    wallet.live?.stale ? 'Updating…' : formatWalletAmount(available(wallet), wallet)
  const others = wallets.filter(wallet => wallet !== primary && wallet.available !== 0n)
  const unbilled = primary.live && !primary.live.stale ? primary.live.pendingUsage : 0n

  // Wallets of different denominations are never added up: each other wallet gets its own line.
  return (
    <Tile
      label={wallets.length > 1 ? 'Credit' : 'Credit balance'}
      value={
        primary.creditUnit && !primary.live?.stale ? (
          <>
            {formatWalletAmount(available(primary), primary, { bare: true })}
            <span className="font-sans text-xs text-muted-foreground">
              {' '}
              {creditUnitName(available(primary), primary.creditUnit)}
            </span>
          </>
        ) : (
          balance(primary)
        )
      }
      meta={
        others.length > 0
          ? others.map(wallet => `${walletTitle(wallet)}: ${balance(wallet)}`).join(' · ')
          : unbilled > 0n
            ? `after ${formatWalletAmount(unbilled, primary)} unbilled usage`
            : walletTitle(primary)
      }
      href="?tab=wallets"
    />
  )
}

/** When the next invoice is paid, as the portal states it: charged on its charge date, else due. */
export const nextPaymentDate = (
  invoice: Pick<UpcomingInvoice, 'chargeDate' | 'dueDate' | 'invoiceDate'>,
  charged: boolean
) => (charged ? invoice.chargeDate : invoice.dueDate) || invoice.invoiceDate

/** What the soonest live subscription's next invoice will charge, computed like on its page. */
const NextChargeTile = ({
  customerId,
  paymentMethod,
}: {
  customerId: string
  paymentMethod?: string
}) => {
  const basePath = useBasePath()
  const live = useQuery(listSubscriptions, {
    customerId,
    status: [SubscriptionStatus.ACTIVE, SubscriptionStatus.TRIALING],
    pagination: { perPage: 20, page: 0 },
  })
  const soonest = (live.data?.subscriptions ?? [])
    .filter(s => s.currentPeriodEnd)
    .sort((a, b) => (a.currentPeriodEnd ?? '').localeCompare(b.currentPeriodEnd ?? ''))[0]
  const next = useQuery(getUpcomingInvoice, soonest ? { subscriptionId: soonest.id } : skipToken, {
    staleTime: SUMMARY_STALE_MS,
    retry: PREVIEW_RETRIES,
  })

  if (live.isLoading) return <Tile label="Next charge" loading />
  if (!soonest) return <Tile label="Next charge" value="—" meta="No active subscription" />
  const invoice = next.data?.invoice
  const href = `${basePath}/subscriptions/${soonest.id}`

  const charged = soonest.chargeAutomatically
  const label = charged ? 'Next charge' : 'Next invoice'
  if (next.isLoading) return <Tile label={label} href={href} loading />
  return invoice ? (
    <Tile
      label={label}
      value={formatMoney(Number(invoice.amountDue), invoice.currency)}
      meta={
        charged
          ? [formatShortDay(nextPaymentDate(invoice, true)), paymentMethod]
              .filter(Boolean)
              .join(' · ')
          : `Due ${formatShortDay(nextPaymentDate(invoice, false))}`
      }
      href={href}
    />
  ) : (
    <Tile
      label={label}
      value="—"
      meta="Couldn't compute the preview here. View it on the subscription."
      href={href}
    />
  )
}
