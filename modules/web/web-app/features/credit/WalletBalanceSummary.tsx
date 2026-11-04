import Decimal from 'decimal.js'

import { WalletStatusBadge, walletHealthKey } from '@/features/credit/WalletStatusBadge'
import { creditUnitName, formatWalletAmount } from '@/features/credit/format'
import { liveUpdatingDescription, walletHealthDescription } from '@/features/credit/walletStatus'
import { useCreditUnit } from '@/features/pricing/CreditUnitPriceFields'
import { currencyPrecision } from '@/lib/data/currency-precision'
import { formatCurrency } from '@/lib/utils/numbers'

import type { Wallet } from '@/rpc/api/wallets/v1/models_pb'

/** "≈ $124.00" for credits, at the customer's agreed price, else the unit's only price. */
const useMoneyEquivalent = (wallet: Wallet, balance: bigint): string | undefined => {
  const unit = useCreditUnit(wallet.creditUnit?.id)
  const label = wallet.creditUnit
  const value =
    wallet.unitPrice ??
    (unit?.prices.length === 1
      ? { currency: unit.prices[0].currency, unitPrice: unit.prices[0].unitPrice }
      : undefined)
  if (!label || !value || balance <= 0n) return undefined
  try {
    const minor = new Decimal(balance.toString())
      .div(new Decimal(10).pow(label.precision))
      .mul(value.unitPrice)
      .mul(new Decimal(10).pow(currencyPrecision(value.currency)))
      .toDecimalPlaces(0)
    return minor.gt(0) ? `≈ ${formatCurrency(BigInt(minor.toFixed()), value.currency)}` : undefined
  } catch {
    return undefined
  }
}

/** What one subscription can spend of the wallet. */
export type ScopedBalance = { balance: bigint; subscription: string }

/** The wallet's name, status and balance (credit on account less unbilled usage), or with
 * `scoped` what one subscription can spend of it. `notes` follow the wallet's own. */
export const WalletBalanceSummary = ({
  wallet,
  title,
  scoped,
  notes: extraNotes = [],
}: {
  wallet: Wallet
  title: string
  scoped?: ScopedBalance
  notes?: string[]
}) => {
  const amount = (minor: bigint) => formatWalletAmount(minor, wallet)
  const live = wallet.live
  const stale = !!live?.stale
  const key = walletHealthKey(wallet.status)
  const balance = scoped ? scoped.balance : live ? live.availableAfterUsage : wallet.available
  const equivalent = useMoneyEquivalent(wallet, balance)
  const notes = scoped
    ? [`What ${scoped.subscription} can spend: its own and the customer-wide credit`]
    : [
        live && !stale && live.uncoveredUsage > 0n
          ? `${amount(live.uncoveredUsage)} beyond credit, billed`
          : undefined,
        wallet.pending > 0n ? `${amount(wallet.pending)} scheduled` : undefined,
        ...extraNotes,
      ].filter(Boolean)

  return (
    <div className="min-w-0 space-y-1">
      <div className="flex flex-wrap items-center gap-2">
        <h3 className="text-sm font-medium text-muted-foreground">{title}</h3>
        <WalletStatusBadge status={wallet.status} stale={stale} />
      </div>
      <div className="text-3xl font-semibold tracking-tight tabular-nums">
        {stale ? (
          <span className="text-muted-foreground">Updating…</span>
        ) : (
          <>
            {formatWalletAmount(balance, wallet, { bare: true })}
            {wallet.creditUnit && (
              <span className="ml-1.5 text-base font-medium text-muted-foreground">
                {creditUnitName(balance, wallet.creditUnit)}
              </span>
            )}
            {equivalent && (
              <span className="ml-2 text-sm font-normal text-muted-foreground">({equivalent})</span>
            )}
          </>
        )}
      </div>
      {stale ? (
        <p className="text-xs text-muted-foreground">{liveUpdatingDescription}</p>
      ) : (
        notes.length > 0 && (
          <p className="text-xs text-muted-foreground tabular-nums">{notes.join(' · ')}</p>
        )
      )}
      {!stale && (key === 'depleted' || key === 'blocked') && (
        <p className={key === 'blocked' ? 'text-xs text-destructive' : 'text-xs text-warning'}>
          {walletHealthDescription[key]}
        </p>
      )}
    </div>
  )
}
