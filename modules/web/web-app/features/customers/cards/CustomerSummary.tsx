import { skipToken } from '@connectrpc/connect-query'
import { Card, Skeleton, cn } from '@md/ui'
import { ReactNode } from 'react'
import { Link } from 'react-router-dom'

import { creditUnitName, formatWalletAmount, walletTitle } from '@/features/credit/format'
import { paymentMethodLabel } from '@/features/credit/paymentMethods'
import { formatShortDay } from '@/features/subscriptions/utils/dates'
import { useBasePath } from '@/hooks/useBasePath'
import { useIsExpressOrganization } from '@/hooks/useIsExpressOrganization'
import { usePermissions } from '@/hooks/usePermissions'
import { useQuery } from '@/lib/connectrpc'
import { formatMoney, parseInstant } from '@/lib/utils/format'
import { getCustomerSummary } from '@/rpc/api/customers/v1/customers-CustomersService_connectquery'
import { SubscriptionStatus } from '@/rpc/api/subscriptions/v1/models_pb'
import {
  getUpcomingInvoice,
  listSubscriptions,
} from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { listCustomerWallets } from '@/rpc/api/wallets/v1/wallets-WalletsService_connectquery'

import type { CustomerMoneyTotal, CustomerReceivables } from '@/rpc/api/customers/v1/customers_pb'
import type { Customer } from '@/rpc/api/customers/v1/models_pb'
import type { UpcomingInvoice } from '@/rpc/api/subscriptions/v1/subscriptions_pb'
import type { Wallet } from '@/rpc/api/wallets/v1/models_pb'

const SUMMARY_STALE_MS = 60_000

export const useCustomerSummary = (customerId: string) =>
  useQuery(getCustomerSummary, { customerId }, { staleTime: SUMMARY_STALE_MS })

const sumText = (totals: CustomerMoneyTotal[]) =>
  totals.map(t => formatMoney(Number(t.amountCents), t.currency)).join(' + ')

const tileCls = 'bg-card rounded-md p-4 flex flex-col gap-1 min-w-0'

const Tile = ({
  label,
  value,
  meta,
  href,
}: {
  label: string
  value: ReactNode
  meta?: ReactNode
  href?: string
}) => {
  const body = (
    <>
      <span className="text-[13px] text-muted-foreground">{label}</span>
      <span className="break-words text-xl tabular-nums text-foreground">{value}</span>
      {meta && <span className="line-clamp-2 text-xs text-muted-foreground">{meta}</span>}
    </>
  )
  return href ? (
    <Link to={href} className="contents">
      <Card className={cn(tileCls, 'hover:border-foreground/20')}>{body}</Card>
    </Link>
  ) : (
    <Card className={tileCls}>{body}</Card>
  )
}

/**
 * What the account is worth, what credit it holds, what it is charged next and what it owes. A
 * tile the caller may not see is left out.
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
  const summary = useCustomerSummary(customerId)
  const data = summary.data

  if (summary.isLoading) {
    return (
      <div className="grid grid-cols-2 gap-3 xl:grid-cols-4">
        {Array.from({ length: 4 }, (_, i) => (
          <Skeleton key={i} height={84} />
        ))}
      </div>
    )
  }
  if (!data) return null

  const receivables = data.receivables
  const overdue = receivables ? sumText(receivables.overdue) : ''
  const converted = !!customer && customer.currency !== data.reportingCurrency
  const defaultMethod = customer?.paymentMethods.find(
    method => method.id === customer.currentPaymentMethodId
  )

  return (
    <div className="grid grid-cols-2 gap-3 xl:grid-cols-4">
      <Tile
        label={converted ? `Recurring · ${data.reportingCurrency}` : 'Recurring'}
        value={
          <>
            {formatMoney(Number(data.mrrCents), data.reportingCurrency)}
            <span className="text-sm text-muted-foreground"> / month</span>
          </>
        }
        meta={
          converted
            ? `Converted to ${data.reportingCurrency}, the reporting currency`
            : (data.term?.planName ?? 'Active subscriptions')
        }
      />
      {canViewCredit && <CreditBalanceTile customerId={customerId} currency={customer?.currency} />}
      {canViewSubscriptions && (
        <NextChargeTile
          customerId={customerId}
          paymentMethod={defaultMethod ? paymentMethodLabel(defaultMethod) : undefined}
        />
      )}
      {receivables && (
        <Tile
          label="Outstanding"
          value={receivables.open.length > 0 ? sumText(receivables.open) : 'Nothing due'}
          meta={overdue ? `${overdue} overdue` : 'Nothing overdue'}
        />
      )}
    </div>
  )
}

/** The plan's credit first (a credit-unit wallet), else the account credit in the customer's currency. */
const CreditBalanceTile = ({ customerId, currency }: { customerId: string; currency?: string }) => {
  const query = useQuery(listCustomerWallets, { customerId }, { staleTime: SUMMARY_STALE_MS })
  const wallets = query.data?.wallets ?? []
  if (query.isLoading) return <Skeleton height={84} />
  if (wallets.length === 0) return null
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
            <span className="text-sm text-muted-foreground">
              {' '}
              {creditUnitName(available(primary), primary.creditUnit)}
            </span>
          </>
        ) : (
          balance(primary)
        )
      }
      meta={
        others.length > 0 ? (
          <span className="flex flex-col">
            {others.map(wallet => (
              <span key={wallet.id}>
                {walletTitle(wallet)}: {balance(wallet)}
              </span>
            ))}
          </span>
        ) : unbilled > 0n ? (
          `after ${formatWalletAmount(unbilled, primary)} unbilled usage`
        ) : (
          walletTitle(primary)
        )
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
  })

  if (live.isLoading || (!!soonest && next.isLoading)) return <Skeleton height={84} />
  if (!soonest) return null
  const invoice = next.data?.invoice
  const href = `${basePath}/subscriptions/${soonest.id}`

  const charged = soonest.chargeAutomatically
  const label = charged ? 'Next charge' : 'Next invoice'
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
      meta="Preview takes longer than usual. Open the subscription."
      href={href}
    />
  )
}

/** "€8,377.60 open · €8,377.60 overdue · last attempt failed 12 Sep · next retry 19 Sep". */
export const ReceivablesLine = ({ receivables }: { receivables: CustomerReceivables }) => {
  const basePath = useBasePath()
  const failedAt = receivables.lastFailureAt ? parseInstant(receivables.lastFailureAt) : undefined
  const retryAt = receivables.nextRetryAt ? parseInstant(receivables.nextRetryAt) : undefined
  const day = (d?: Date) =>
    d && !Number.isNaN(d.getTime())
      ? d.toLocaleDateString(undefined, { day: 'numeric', month: 'short' })
      : undefined
  const parts: ReactNode[] = [
    receivables.open.length > 0 ? `${sumText(receivables.open)} open` : 'Nothing open',
    receivables.overdue.length > 0 && `${sumText(receivables.overdue)} overdue`,
    day(failedAt) &&
      (receivables.lastFailureInvoiceId ? (
        <Link
          key="failure"
          to={`${basePath}/invoices/${receivables.lastFailureInvoiceId}`}
          className="hover:underline"
        >
          last attempt failed {day(failedAt)}
          {receivables.lastFailureReason ? ` (${receivables.lastFailureReason})` : ''}
        </Link>
      ) : (
        `last attempt failed ${day(failedAt)}`
      )),
    day(retryAt) && `next retry ${day(retryAt)}`,
  ].filter(Boolean)

  return (
    <p className="text-[13px] text-muted-foreground">
      {parts.map((p, i) => (
        <span key={i}>
          {i > 0 && ' · '}
          {p}
        </span>
      ))}
    </p>
  )
}
