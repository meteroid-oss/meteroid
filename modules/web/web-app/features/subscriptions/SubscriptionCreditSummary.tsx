import { Button, Tooltip, TooltipContent, TooltipTrigger } from '@md/ui'
import { Plus } from 'lucide-react'
import { Fragment, type ReactNode } from 'react'
import { Link } from 'react-router-dom'

import { slotItems } from '@/edition'
import { WalletStatusBadge } from '@/features/credit/WalletStatusBadge'
import { formatWalletAmount, walletTitle } from '@/features/credit/format'
import { SubscriptionSectionCard } from '@/features/subscriptions/SubscriptionSectionCard'
import {
  creditNote,
  formatBalance,
  idleWalletDetail,
  isIdleWallet,
  ledgerStatus,
  packRollover,
  servesOnly,
  usableCredit,
  walletlessCredit,
} from '@/features/subscriptions/subscriptionOverview'
import { creditUnitNames, packCredits, toMinor } from '@/features/subscriptions/subscriptionPrice'
import {
  creditSplitLine,
  periodUsageFor,
  subscriptionCreditSplit,
} from '@/features/subscriptions/subscriptionWallets'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { currencyPrecision } from '@/lib/data/currency-precision'
import { getUpcomingInvoice } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { WalletStatus } from '@/rpc/api/wallets/v1/models_pb'

import type { SubscriptionCreditRowContext } from '@/edition/types'
import type { WalletDenomination } from '@/features/credit/format'
import type { CreditUnitNames, PricedFee } from '@/features/subscriptions/subscriptionPrice'
import type { Subscription } from '@/rpc/api/subscriptions/v1/models_pb'
import type { PendingInvoiceCredit } from '@/rpc/api/subscriptions/v1/subscriptions_pb'
import type { Wallet } from '@/rpc/api/wallets/v1/models_pb'

// Without a module's live view, no subscription is known to share the credit.
const useNoSharing = (): string[] => []

const packKey = (wallet: Wallet) => wallet.creditUnit?.id ?? wallet.currency

export const SubscriptionCreditSummary = ({
  subscription,
  wallets,
  fees,
  creditUnits,
  pendingCredit,
  others,
  canAddCredit,
  onAddCredit,
}: {
  subscription: Subscription
  wallets: Wallet[]
  fees: PricedFee[]
  creditUnits: Record<string, CreditUnitNames>
  pendingCredit: PendingInvoiceCredit[]
  /** The customer's other live subscriptions. */
  others: Subscription[]
  canAddCredit: boolean
  onAddCredit: (wallet: Wallet) => void
}) => {
  const basePath = useBasePath()
  const live = Boolean(subscription.currentPeriodStart) && Boolean(subscription.currentPeriodEnd)
  const upcoming = useQuery(
    getUpcomingInvoice,
    { subscriptionId: subscription.id },
    { enabled: live && wallets.length > 0 }
  )
  const creditLive = slotItems('subscriptionCredit.live')[0]
  const useSharedWith = creditLive?.useSharedWith ?? useNoSharing
  const shared = new Set(useSharedWith(subscription.id))
  const packs = packCredits(fees, subscription.currency).filter(p => !p.once)
  const precisionOf = (key: string, creditUnitId?: string) =>
    creditUnitId
      ? (creditUnits[creditUnitId]?.precision ??
        packs.find(p => p.key === key)?.creditUnit?.precision ??
        0)
      : currencyPrecision(subscription.currency)
  const walletless = walletlessCredit(
    packs.map(p => ({
      key: p.key,
      creditUnitId: p.creditUnit?.creditUnitId,
      total: toMinor(p.total, precisionOf(p.key, p.creditUnit?.creditUnitId)),
    })),
    pendingCredit,
    wallets
  )
  if (wallets.length === 0 && walletless.length === 0) return null

  const single = wallets.length === 1
  const asOf = new Date()
  const invoice = upcoming.data?.invoice
  const Row = creditLive?.Row ?? CreditRow
  const sharedWith = others.filter(other => shared.has(other.id))

  const expiryContext = (wallet: Wallet) => {
    const precision = wallet.creditUnit?.precision ?? currencyPrecision(wallet.currency)
    const own = packs.filter(p => p.key === packKey(wallet))
    const refill = own.reduce((sum, p) => sum + toMinor(p.total, precision), 0n)
    return {
      refill: own.length > 0 ? refill : undefined,
      rollover: packRollover(fees, subscription.currency, packKey(wallet), precision),
    }
  }

  return (
    <SubscriptionSectionCard
      title="Credit"
      subtitle={
        sharedWith.length > 0 ? (
          <>
            Shared with{' '}
            {sharedWith.map((other, i) => (
              <Fragment key={other.id}>
                {i > 0 && ', '}
                <Link
                  to={`${basePath}/subscriptions/${other.id}`}
                  className="text-brand hover:underline"
                >
                  {other.planName}
                </Link>
              </Fragment>
            ))}
          </>
        ) : undefined
      }
      aside={
        <div className="flex items-center gap-3">
          <Link
            to={`${basePath}/customers/${subscription.customerId}?tab=wallets`}
            className="text-sm text-brand hover:underline"
          >
            View credit →
          </Link>
          {single && canAddCredit && (
            <Button size="sm" variant="outline" onClick={() => onAddCredit(wallets[0])}>
              <Plus size={14} /> Add credit
            </Button>
          )}
        </div>
      }
    >
      <div className="divide-y divide-border">
        {wallets.map(wallet => {
          const periodUsage = invoice ? periodUsageFor(wallet, invoice) : undefined
          const pending = pendingCredit.filter(p =>
            wallet.creditUnit
              ? p.creditUnitId === wallet.creditUnit.id
              : !p.creditUnitId && p.currency === wallet.currency
          )
          return (
            <Row
              key={wallet.id}
              wallet={wallet}
              subscriptionId={subscription.id}
              periodUsage={periodUsage}
              periodEnd={subscription.currentPeriodEnd}
              asOf={asOf}
              pending={pending}
              expiry={expiryContext(wallet)}
              addCredit={!single && canAddCredit ? () => onAddCredit(wallet) : undefined}
            />
          )
        })}
        {walletless.map(row => {
          const pack = packs.find(p => p.key === row.key)
          const names = row.creditUnitId
            ? (creditUnits[row.creditUnitId] ??
              (pack?.creditUnit ? creditUnitNames(pack.creditUnit, creditUnits) : undefined))
            : undefined
          const code = pack?.creditUnit?.code ?? row.pending[0]?.currency ?? row.key
          const denomination = names
            ? { currency: code, creditUnit: names }
            : { currency: subscription.currency }
          return (
            <div
              key={row.key}
              className="flex flex-wrap items-center gap-x-4 gap-y-1 px-4 py-3 text-sm"
            >
              <span className="font-medium text-foreground">
                {names ? names.pluralName : `Usage credit · ${subscription.currency}`}
              </span>
              <span className="text-muted-foreground">
                No credit on account yet
                {row.pending.length === 0 && row.nextAdds !== undefined
                  ? ` · the next invoice adds ${formatWalletAmount(row.nextAdds, denomination)}`
                  : ''}
              </span>
              <PendingCredit credits={row.pending} denomination={denomination} />
            </div>
          )
        })}
      </div>
    </SubscriptionSectionCard>
  )
}

/** "+€400.00 once 4 invoices are paid", the invoices listed on hover. */
export const PendingCredit = ({
  credits,
  denomination,
}: {
  credits: PendingInvoiceCredit[]
  denomination: WalletDenomination
}) => {
  const basePath = useBasePath()
  if (credits.length === 0) return null
  const total = formatWalletAmount(
    credits.reduce((sum, credit) => sum + credit.amount, 0n),
    denomination
  )
  const link = (credit: PendingInvoiceCredit) => (
    <Link
      key={credit.invoiceId}
      to={`${basePath}/invoices/${credit.invoiceId}`}
      className="text-brand hover:underline"
    >
      {credit.invoiceNumber}
    </Link>
  )
  if (credits.length === 1) {
    return (
      <span className="text-xs text-muted-foreground">
        +{total} once {link(credits[0])} is paid
      </span>
    )
  }
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="cursor-help text-xs text-muted-foreground underline decoration-dotted underline-offset-2">
          +{total} once {credits.length} invoices are paid
        </span>
      </TooltipTrigger>
      <TooltipContent className="flex flex-col gap-1">
        {credits.map(credit => (
          <span key={credit.invoiceId} className="flex justify-between gap-4">
            {link(credit)}
            <span className="tabular-nums">{formatWalletAmount(credit.amount, denomination)}</span>
          </span>
        ))}
      </TooltipContent>
    </Tooltip>
  )
}

/** The wallet's credit on account the subscription can use, as the ledger holds it. */
const CreditRow = ({
  wallet,
  subscriptionId,
  periodUsage,
  periodEnd,
  pending,
  expiry,
  addCredit,
}: SubscriptionCreditRowContext) => {
  const title = walletTitle(wallet)
  const own = servesOnly(wallet, subscriptionId)
  const balance = usableCredit(wallet, subscriptionId)
  // Credit only another subscription can use expires or arrives on its page, not here.
  const usable = {
    ...wallet,
    grants: wallet.grants.filter(g => !g.subscriptionId || g.subscriptionId === subscriptionId),
  }
  const note = creditNote(usable, periodEnd, expiry)
  const split = creditSplitLine(subscriptionCreditSplit(wallet, subscriptionId), minor =>
    formatWalletAmount(minor, wallet)
  )
  const button = addCredit && (
    <Button size="sm" variant="outline" onClick={addCredit}>
      <Plus size={14} /> Add credit
    </Button>
  )

  if (own && pending.length === 0 && isIdleWallet(wallet, periodUsage, note)) {
    const detail = idleWalletDetail(wallet)
    return (
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 px-4 py-3 text-sm">
        <span className="font-medium text-foreground md:w-44 md:shrink-0">{title}</span>
        <span className="flex-1 tabular-nums">
          <BalanceDetails details={[split]}>
            <span className="font-semibold text-foreground">{formatBalance(balance, wallet)}</span>
          </BalanceDetails>
          {detail && <span className="text-muted-foreground"> · {detail}</span>}
        </span>
        {button}
      </div>
    )
  }

  const status = ledgerStatus(wallet, subscriptionId)
  // Credit bought on an unpaid invoice explains an empty wallet better than "Depleted".
  const badge =
    pending.length > 0 && status === WalletStatus.DEPLETED ? WalletStatus.HEALTHY : status

  return (
    <div className="flex flex-col gap-1 px-4 py-3 text-sm md:grid md:grid-cols-[11rem_minmax(0,1fr)_auto] md:items-center md:gap-x-4">
      <span className="font-medium text-foreground">{title}</span>
      <div className="flex flex-wrap items-center gap-2">
        <BalanceDetails
          details={[balance <= 0n ? 'No credit on account' : undefined, split, note]}
        >
          <span className="font-semibold text-foreground tabular-nums">
            {formatBalance(balance, wallet)}
          </span>
        </BalanceDetails>
        <WalletStatusBadge status={badge} />
        <PendingCredit credits={pending} denomination={wallet} />
      </div>
      <div className="flex items-center gap-3 md:justify-end">{button}</div>
    </div>
  )
}

/** The balance, with how it adds up, its split and its expiry on hover. */
export const BalanceDetails = ({
  details,
  children,
}: {
  details: (string | false | null | undefined)[]
  children: ReactNode
}) => {
  const lines = details.filter((line): line is string => !!line)
  if (lines.length === 0) return children
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="cursor-help underline decoration-muted-foreground decoration-dotted underline-offset-4">
          {children}
        </span>
      </TooltipTrigger>
      <TooltipContent className="max-w-80 space-y-1 tabular-nums">
        {lines.map(line => (
          <p key={line}>{line}</p>
        ))}
      </TooltipContent>
    </Tooltip>
  )
}
