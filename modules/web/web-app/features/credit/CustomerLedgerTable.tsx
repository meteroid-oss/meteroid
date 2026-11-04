import { ConnectError } from '@connectrpc/connect'
import { useMutation } from '@connectrpc/connect-query'
import { Badge, Button, Dialog, DialogContent, DialogHeader, DialogTitle, cn } from '@md/ui'
import { Download } from 'lucide-react'
import { useMemo, useState } from 'react'
import { Link } from 'react-router-dom'
import { toast } from 'sonner'

import Pagination from '@/components/table/CustomTable/components/Pagination'
import { SimpleTable } from '@/components/table/SimpleTable'
import { formatWalletAmount, walletTitle } from '@/features/credit/format'
import { formatDate, ledgerMovementLabel } from '@/features/credit/labels'
import { LEDGER_PAGE_SIZE, customerLedgerCsvUrl } from '@/features/credit/ledgerExport'
import { useBasePath } from '@/hooks/useBasePath'
import { useQuery } from '@/lib/connectrpc'
import { env } from '@/lib/env'
import { CreditGrantSource, CreditGrantType, LedgerEntryType } from '@/rpc/api/wallets/v1/models_pb'
import {
  exportCustomerLedger,
  listCustomerLedgerEntries,
} from '@/rpc/api/wallets/v1/wallets-WalletsService_connectquery'

import type { ScopedWalletBalance } from '@/edition/types'
import type { CustomerLedgerEntry, Wallet } from '@/rpc/api/wallets/v1/models_pb'
import type { ColumnDef } from '@tanstack/react-table'

const denomination = (row: Pick<CustomerLedgerEntry, 'currency' | 'creditUnit'>) => ({
  currency: row.currency,
  creditUnit: row.creditUnit,
})

const walletLabel = (row: CustomerLedgerEntry) => row.creditUnit?.pluralName ?? row.currency

// Labels of these sources already say whether the credit is promotional.
const SELF_DESCRIBED = new Set([
  CreditGrantSource.INCLUDED,
  CreditGrantSource.ROLLOVER,
  CreditGrantSource.PURCHASE,
  CreditGrantSource.COUPON,
  CreditGrantSource.MANUAL,
  CreditGrantSource.API,
])

// Names the system gives a grant that only repeat its movement label.
const GENERIC_GRANT_NAMES = new Set([
  'purchase credits',
  'automatic top-up',
  'rolled over from expiring credit',
])

/** The grant's name, unless it only repeats the movement label. */
const grantDetail = (row: CustomerLedgerEntry): string | undefined => {
  const name = row.grantName.trim().toLowerCase()
  if (!name || GENERIC_GRANT_NAMES.has(name) || name === row.label.trim().toLowerCase()) {
    return undefined
  }
  return row.grantName
}

/** "Monthly credit (free)", the part after the movement label. */
export const movementDetail = (row: CustomerLedgerEntry): string | undefined => {
  const entry = row.entry
  if (!entry) return undefined
  switch (entry.type) {
    case LedgerEntryType.GRANTED: {
      const name = grantDetail(row)
      const promotional =
        row.grantType === CreditGrantType.PROMOTIONAL && !SELF_DESCRIBED.has(row.grantSource)
      return name && `${name}${promotional ? ' (free)' : ''}`
    }
    case LedgerEntryType.EXPIRED:
      return grantDetail(row)
    case LedgerEntryType.REMOVED:
    case LedgerEntryType.RETURNED:
      // A removal the label already names (e.g. "Removed: plan change") shows its credit instead.
      if (entry.reason && entry.reason === row.label) return row.grantName || undefined
      return entry.reason || undefined
    default:
      return undefined
  }
}

/** The wallet's live usage not invoiced yet: shown above its movements, so the balances add up. */
export type PendingUsageRow = {
  pending: true
  amount: bigint
  balanceAfter: bigint
} & Pick<Wallet, 'currency' | 'creditUnit'>

type LedgerRow = CustomerLedgerEntry | PendingUsageRow

const isPending = (row: LedgerRow): row is PendingUsageRow => 'pending' in row

export const pendingUsageRow = (
  wallet: Pick<Wallet, 'live' | 'currency' | 'creditUnit'>
): PendingUsageRow | undefined => {
  const live = wallet.live
  if (!live || live.stale || live.pendingUsage <= 0n) return undefined
  return {
    pending: true,
    amount: -live.pendingUsage,
    balanceAfter: live.availableAfterUsage,
    currency: wallet.currency,
    creditUnit: wallet.creditUnit,
  }
}

/**
 * The subscription's own usage not invoiced yet that credit pays, off the statement's last
 * balance. Other subscriptions' unbilled use of shared credit and credit it cannot use stay in it.
 */
export const scopedPendingRow = (
  wallet: Pick<Wallet, 'currency' | 'creditUnit'>,
  balance: Pick<ScopedWalletBalance, 'unbilled' | 'uncovered'> | undefined,
  lastBalance: bigint
): PendingUsageRow | undefined => {
  if (!balance) return undefined
  const paid = balance.unbilled - balance.uncovered
  if (paid <= 0n) return undefined
  return {
    pending: true,
    amount: -paid,
    balanceAfter: lastBalance - paid,
    currency: wallet.currency,
    creditUnit: wallet.creditUnit,
  }
}

type LedgerWallet = Pick<Wallet, 'id' | 'currency' | 'creditUnit' | 'live'>

/** One subscription's view of a wallet: the movements of the credit it can spend. */
export type LedgerScope = {
  subscriptionId: string
  /** The subscription's view of the wallet, when known. */
  balance?: Pick<ScopedWalletBalance, 'unbilled' | 'uncovered'>
}

/**
 * The statement of one wallet, or of all the customer's wallets with a Wallet column. With
 * `limit`, only the latest movements show, and the whole statement opens in a dialog. With
 * `scope`, only the movements of the credit one subscription can spend, balances counting those.
 */
export const CustomerLedgerTable = ({
  customerId,
  wallet,
  scope,
  limit,
}: {
  customerId: string
  wallet?: LedgerWallet
  scope?: LedgerScope
  limit?: number
}) => {
  const basePath = useBasePath()
  const [page, setPage] = useState(0)
  const [showAll, setShowAll] = useState(false)
  const walletId = wallet?.id
  const scoped = !!wallet
  const subscriptionId = scope?.subscriptionId
  const scopeBalance = scope?.balance
  const query = useQuery(listCustomerLedgerEntries, {
    customerId,
    walletId,
    subscriptionId,
    pagination: limit ? { page: 0, perPage: limit } : { page, perPage: LEDGER_PAGE_SIZE },
  })
  const exportLedger = useMutation(exportCustomerLedger)
  const entries = query.data?.entries
  const pending = useMemo(() => {
    if (!wallet || page !== 0) return undefined
    if (!subscriptionId) return pendingUsageRow(wallet)
    return entries
      ? scopedPendingRow(wallet, scopeBalance, entries[0]?.entry?.balanceAfter ?? 0n)
      : undefined
  }, [wallet, page, subscriptionId, scopeBalance, entries])
  const rows = useMemo<LedgerRow[] | undefined>(
    () => (entries && pending ? [pending, ...entries] : entries),
    [entries, pending]
  )

  const columns = useMemo<ColumnDef<LedgerRow>[]>(
    () => [
      {
        header: 'Date',
        id: 'date',
        cell: ({ row }) =>
          isPending(row.original) ? (
            <Badge size="sm" variant="ghost">
              Pending
            </Badge>
          ) : (
            <span className="whitespace-nowrap">{formatDate(row.original.entry?.effectiveAt)}</span>
          ),
      },
      ...(scoped
        ? []
        : [
            {
              header: 'Wallet',
              id: 'wallet',
              cell: ({ row }: { row: { original: LedgerRow } }) =>
                isPending(row.original) ? null : (
                  <span className="whitespace-nowrap">{walletLabel(row.original)}</span>
                ),
            } satisfies ColumnDef<LedgerRow>,
          ]),
      {
        header: 'Description',
        id: 'description',
        cell: ({ row }) =>
          isPending(row.original) ? (
            <span className="font-medium text-muted-foreground">Usage not invoiced yet</span>
          ) : (
            <LedgerDescription row={row.original} basePath={basePath} />
          ),
      },
      {
        header: () => <div className="text-right">Amount</div>,
        id: 'amount',
        cell: ({ row }) => {
          const original = row.original
          const amount = isPending(original) ? original.amount : (original.entry?.amount ?? 0n)
          if (isPending(original) || isCarriedOver(original)) {
            return (
              <div className="text-right whitespace-nowrap tabular-nums text-muted-foreground">
                {isPending(original)
                  ? formatWalletAmount(amount, denomination(original), { signed: true })
                  : `${formatWalletAmount(amount, denomination(original))} carried over`}
              </div>
            )
          }
          return (
            <div
              className={
                amount >= 0n
                  ? 'text-right whitespace-nowrap tabular-nums text-success'
                  : 'text-right whitespace-nowrap tabular-nums'
              }
            >
              {formatWalletAmount(amount, denomination(original), { signed: true })}
            </div>
          )
        },
      },
      ...(scoped
        ? [
            {
              header: () => <div className="text-right">Balance</div>,
              id: 'balance',
              cell: ({ row }: { row: { original: LedgerRow } }) => {
                const original = row.original
                return (
                  <div
                    className={cn(
                      'text-right whitespace-nowrap tabular-nums',
                      isPending(original) && 'text-muted-foreground'
                    )}
                  >
                    {formatWalletAmount(
                      isPending(original)
                        ? original.balanceAfter
                        : (original.entry?.balanceAfter ?? 0n),
                      denomination(original)
                    )}
                  </div>
                )
              },
            } satisfies ColumnDef<LedgerRow>,
          ]
        : []),
    ],
    [basePath, scoped]
  )

  if (query.isError) {
    return (
      <div className="space-y-2" role="alert">
        <p className="text-xs text-destructive">The ledger could not be loaded.</p>
        <Button variant="secondary" size="sm" onClick={() => void query.refetch()}>
          Retry
        </Button>
      </div>
    )
  }

  const totalCount = query.data?.paginationMeta?.totalItems ?? 0
  const truncated = limit !== undefined && totalCount > limit

  // The server streams the statement; the link it signs lasts a few minutes.
  const exportCsv = async () => {
    try {
      const { token } = await exportLedger.mutateAsync({ customerId, walletId, subscriptionId })
      const link = document.createElement('a')
      link.href = customerLedgerCsvUrl(env.meteroidRestApiUri, customerId, token)
      link.click()
    } catch (error) {
      toast.error(ConnectError.from(error).rawMessage || 'The ledger could not be exported')
    }
  }

  return (
    <div className="space-y-2">
      <div className="overflow-x-auto">
        <SimpleTable
          columns={columns}
          data={rows}
          isLoading={query.isLoading}
          emptyMessage="No movements yet."
          containerClassName="min-w-[520px] text-xs"
          headTrClasses="[&_th]:text-xs"
        />
      </div>
      {totalCount > 0 && (
        <div className="flex flex-wrap items-center justify-between gap-2">
          {truncated ? (
            <Button
              variant="link"
              size="sm"
              className="h-auto p-0 text-xs"
              onClick={() => setShowAll(true)}
            >
              View all {totalCount} movements
            </Button>
          ) : (
            <span />
          )}
          <Button
            size="sm"
            variant="ghost"
            disabled={exportLedger.isPending}
            onClick={() => void exportCsv()}
          >
            <Download size={14} /> {exportLedger.isPending ? 'Exporting…' : 'Export CSV'}
          </Button>
        </div>
      )}
      {showAll && (
        <Dialog open onOpenChange={setShowAll}>
          <DialogContent className="max-h-[85vh] max-w-4xl overflow-y-auto">
            <DialogHeader>
              <DialogTitle>
                {wallet ? `Credit ledger · ${walletTitle(wallet)}` : 'Credit ledger'}
              </DialogTitle>
            </DialogHeader>
            <CustomerLedgerTable customerId={customerId} wallet={wallet} scope={scope} />
          </DialogContent>
        </Dialog>
      )}
      {limit === undefined && (
        <div className="flex justify-end">
          <Pagination
            variant="footer"
            pagination={{ pageIndex: page, pageSize: LEDGER_PAGE_SIZE }}
            setPagination={next => {
              const value =
                typeof next === 'function'
                  ? next({ pageIndex: page, pageSize: LEDGER_PAGE_SIZE })
                  : next
              setPage(value.pageIndex)
            }}
            totalCount={totalCount}
            isLoading={query.isLoading}
          />
        </div>
      )}
    </div>
  )
}

const LedgerDescription = ({ row, basePath }: { row: CustomerLedgerEntry; basePath: string }) => {
  const entry = row.entry
  if (!entry) return null
  const detail = movementDetail(row)
  const invoice = entry.invoiceId && (
    <Link className="underline" to={`${basePath}/invoices/${entry.invoiceId}`}>
      {row.invoiceNumber ?? 'invoice'}
    </Link>
  )
  const creditNote = entry.creditNoteId && (
    <Link className="underline" to={`${basePath}/credit-notes/${entry.creditNoteId}`}>
      {row.creditNoteNumber ?? 'credit note'}
    </Link>
  )
  const applied = entry.type === LedgerEntryType.APPLIED

  return (
    <div className="min-w-0">
      <span className="font-medium">{ledgerMovementLabel(row)}</span>
      {applied && invoice && <> {invoice}</>}
      {detail && <span className="text-muted-foreground"> · {detail}</span>}
      {!applied && invoice && <span className="text-muted-foreground"> · {invoice}</span>}
      {creditNote && <span className="text-muted-foreground"> · {creditNote}</span>}
      {entry.reference && <span className="text-muted-foreground"> · {entry.reference}</span>}
    </div>
  )
}

// The statement leaves out the carry-out of a rollover: its funding row moves value between
// grants, so the balance does not change on it.
const isCarriedOver = (row: CustomerLedgerEntry) =>
  row.entry?.type === LedgerEntryType.GRANTED && row.grantSource === CreditGrantSource.ROLLOVER
