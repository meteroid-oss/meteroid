import { create } from '@bufbuild/protobuf'
import { AlertTriangle, Plus } from 'lucide-react'
import { Fragment, useEffect, useState } from 'react'
import { useSearchParams } from 'react-router-dom'

import { Slot } from '@/edition'
import { productScopeLabel } from '@/features/credit/ProductScopeBadge'
import { topUpPresets } from '@/features/credit/amounts'
import { rollsOver } from '@/features/credit/format'
import { formatCredits } from '@/features/credit/unitSale'
import { liveUpdatingDescription, liveUpdatingLabel } from '@/features/credit/walletStatus'
import { formatCurrency } from '@/lib/utils/numbers'
import { WalletSchema, WalletStatus } from '@/rpc/portal/wallets/v1/models_pb'

import { RedeemCodeCard } from '../RedeemCodeCard'
import { usePortalNav } from '../context'
import { cadenceAdjective, clampPct, money, pmLabel } from '../format'
import { useCreditOverview, useSelfService } from '../hooks'
import { TopUpModal, hasPendingTopUp } from '../modals/TopUpModal'
import { TOP_UP_RETURN_PARAM, returnedWallet } from '../modals/topUpCheckout'
import {
  Card,
  CenterState,
  Eyebrow,
  LinkButton,
  Meter,
  Mono,
  PanelCard,
  PButton,
  Pill,
  Spinner,
} from '../primitives'
import {
  boundaryDate,
  boundaryDay,
  creditCount,
  dayTime,
  grantKindLabel,
  walletTitle,
} from '../walletFormat'

import { PortalAlerts } from './AlertBanner'
import { CreditGroups } from './CreditGroups'
import { UsageFilterTag } from './UsageFilterTag'
import { WalletStatement } from './WalletStatement'
import { contactName, creditPaysLine, roundCreditPresets } from './creditCopy'

import type { StatusBadge } from '../format'
import type { CustomerPaymentMethod } from '@/rpc/api/customers/v1/models_pb'
import type { CreditGrantView, Wallet, WalletOverview } from '@/rpc/portal/wallets/v1/models_pb'
import type { ReactNode } from 'react'

type LoadedOverview = WalletOverview & { wallet: Wallet }

// A healthy wallet carries no badge: only states the customer may need to act on.
const STATUS_BADGE: Record<WalletStatus, StatusBadge | undefined> = {
  [WalletStatus.UNSPECIFIED]: undefined,
  [WalletStatus.HEALTHY]: undefined,
  [WalletStatus.LOW]: { label: 'Low balance', tone: 'warn' },
  [WalletStatus.DEPLETED]: { label: 'Used up', tone: 'warn' },
  [WalletStatus.USAGE_BLOCKED]: { label: 'Balance at 0', tone: 'danger' },
}

const addCreditAnchor = (walletId: string) => `mtp-add-credit-${walletId}`

/** Credit units print as bare counts (the eyebrow names the unit); money with its symbol. */
const formatterFor = (wallet: Wallet) => (minor: bigint) =>
  wallet.creditUnit ? creditCount(minor, wallet.creditUnit) : formatCurrency(minor, wallet.currency)

const shortFormatterFor = (wallet: Wallet) => (minor: bigint) =>
  wallet.creditUnit
    ? creditCount(minor, wallet.creditUnit)
    : money(minor, wallet.currency, { compact: true })

const CardGlyph = () => (
  <svg
    width="18"
    height="18"
    viewBox="0 0 24 24"
    fill="none"
    stroke="var(--mtp-text-3)"
    strokeWidth="1.8"
    aria-hidden
  >
    <rect x="2" y="5" width="20" height="14" rx="2.5" />
    <path d="M2 10h20" />
  </svg>
)

const Fact = ({ label, value, sub }: { label: string; value: string; sub: string }) => (
  <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
    <span style={{ fontSize: 12.5, color: 'var(--mtp-text-2)' }}>{label}</span>
    <span style={{ fontSize: 14, fontWeight: 500 }}>{value}</span>
    <span style={{ fontSize: 12.5, color: 'var(--mtp-text-2)' }}>{sub}</span>
  </div>
)

const Facts = ({ overview }: { overview: LoadedOverview }) => {
  const { wallet, nextCredit, totals, nextExpiry } = overview
  const fmt = formatterFor(wallet)
  const facts: ReactNode[] = []
  if (nextCredit) {
    facts.push(
      <Fact
        key="next"
        label="Next credit"
        value={`+${fmt(nextCredit.amount)} on ${boundaryDay(nextCredit.date)}`}
        sub={
          nextCredit.price > 0n
            ? `with your ${money(nextCredit.price, nextCredit.priceCurrency)} ${cadenceAdjective(nextCredit.cadence)} payment${nextCredit.priceExcludesTax ? ' plus tax' : ''}`
            : 'included with your plan'
        }
      />
    )
  }
  if (totals && totals.added > 0n) {
    facts.push(
      <Fact
        key="used"
        label={`${totals.since ? `Used since ${boundaryDay(totals.since)}` : 'Used so far'}${wallet.creditUnit ? '' : ', incl. tax'}`}
        value={`${fmt(totals.used)} of ${fmt(totals.added)}`}
        sub={totals.expired > 0n ? `${fmt(totals.expired)} expired` : 'nothing expired'}
      />
    )
  }
  if (nextExpiry?.expiresAt) {
    facts.push(
      <Fact
        key="expiry"
        label="Next expiry"
        value={boundaryDate(nextExpiry.expiresAt)}
        sub={
          nextExpiry.rolloverAmount !== undefined
            ? `up to ${fmt(nextExpiry.rolloverAmount)} ${rollsOver(nextExpiry.rolloverAmount, wallet.creditUnit)}`
            : `${fmt(nextExpiry.remaining)} left on that credit`
        }
      />
    )
  }
  if (facts.length === 0) return null
  return (
    <div
      style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(auto-fit, minmax(200px, 1fr))',
        gap: 12,
        paddingTop: 14,
        borderTop: '1px solid var(--mtp-border)',
      }}
    >
      {facts}
    </div>
  )
}

const BalanceCard = ({
  overview,
  canTopUp,
  footer,
  onTopUp,
}: {
  overview: LoadedOverview
  canTopUp: boolean
  /** After the Add credit button. */
  footer?: ReactNode
  onTopUp: () => void
}) => {
  const { wallet } = overview
  const fmt = formatterFor(wallet)
  const stale = wallet.liveStale
  const badge = stale ? undefined : STATUS_BADGE[wallet.status]
  return (
    <Card style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
        <Eyebrow>{walletTitle(wallet)}</Eyebrow>
        {badge && <Pill badge={badge} />}
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        <Mono style={{ fontSize: 34, fontWeight: 600, letterSpacing: '-0.02em' }}>
          {stale ? (
            <span style={{ color: 'var(--mtp-text-3)' }}>{liveUpdatingLabel}</span>
          ) : (
            fmt(wallet.availableAfterUsage ?? wallet.available)
          )}
        </Mono>
        {stale ? (
          <span style={{ fontSize: 13, color: 'var(--mtp-text-2)' }}>
            {liveUpdatingDescription}
          </span>
        ) : (
          <span style={{ fontSize: 13.5 }}>
            {fmt(wallet.available)} credit on account
            {wallet.pendingUsage !== undefined &&
              ` − ${fmt(wallet.pendingUsage)} unbilled usage${wallet.creditUnit ? '' : ' incl. tax'}`}
            {wallet.pendingUsage !== undefined && wallet.asOf && (
              <span style={{ color: 'var(--mtp-text-2)' }}> · updated {dayTime(wallet.asOf)}</span>
            )}
          </span>
        )}
        {!stale && wallet.uncoveredUsage > 0n && (
          <span style={{ fontSize: 13, color: 'var(--mtp-text-2)' }}>
            {fmt(wallet.uncoveredUsage)} of usage beyond your credit is added to your next invoice.
          </span>
        )}
      </div>
      <CreditGroups wallet={wallet} />
      {!stale && <Facts overview={overview} />}
      {canTopUp && (
        <div>
          <PButton variant="secondary" size="sm" onClick={onTopUp}>
            <Plus size={13} /> Add credit
          </PButton>
        </div>
      )}
      {footer}
    </Card>
  )
}

/** Shown while usage is paused: the quickest way back is to pay for credit right here. */
const AddCreditCard = ({
  wallet,
  paymentMethod,
  onPay,
  onChangePaymentMethod,
}: {
  wallet: Wallet
  paymentMethod?: CustomerPaymentMethod
  onPay: (amount?: bigint) => void
  onChangePaymentMethod: () => void
}) => {
  const unit = wallet.creditUnit
  const salePrice = wallet.salePrice
  const presets = unit
    ? salePrice
      ? roundCreditPresets(salePrice.unitPrice, unit.precision, salePrice.currency)
      : []
    : topUpPresets(wallet.currency)
  const short = shortFormatterFor(wallet)
  const [choice, setChoice] = useState<bigint | 'other'>(
    () =>
      presets.find(amount => amount > wallet.uncoveredUsage) ??
      presets[presets.length - 1] ??
      'other'
  )
  const chip = (selected: boolean) => ({
    minWidth: 88,
    height: 44,
    padding: '0 14px',
    borderRadius: 'var(--mtp-r-ctrl)',
    border: `1px solid ${selected ? 'var(--mtp-accent)' : 'var(--mtp-border-2)'}`,
    background: selected ? 'var(--mtp-accent-weak)' : 'transparent',
    color: 'var(--mtp-text)',
    fontFamily: 'inherit',
    fontSize: 13.5,
    fontWeight: 600,
    cursor: 'pointer',
  })
  const headingId = `${addCreditAnchor(wallet.id)}-h`
  return (
    <section id={addCreditAnchor(wallet.id)} aria-labelledby={headingId}>
      <Card style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
        <h2 id={headingId} style={{ margin: 0, fontSize: 15, fontWeight: 600 }}>
          Add credit
        </h2>
        <fieldset
          style={{ border: 0, margin: 0, padding: 0, display: 'flex', flexWrap: 'wrap', gap: 8 }}
        >
          <legend style={{ padding: 0, marginBottom: 8, fontSize: 13, color: 'var(--mtp-text-2)' }}>
            Amount
          </legend>
          {presets.map(amount => (
            <button
              key={amount.toString()}
              type="button"
              className="mtp-btn"
              aria-pressed={choice === amount}
              onClick={() => setChoice(amount)}
              style={{ ...chip(choice === amount), fontFamily: 'var(--mtp-mono)' }}
            >
              {short(amount)}
            </button>
          ))}
          <button
            type="button"
            className="mtp-btn"
            aria-pressed={choice === 'other'}
            onClick={() => setChoice('other')}
            style={{ ...chip(choice === 'other'), fontWeight: 500, color: 'var(--mtp-text-2)' }}
          >
            Other
          </button>
        </fieldset>
        <div
          style={{
            display: 'flex',
            flexWrap: 'wrap',
            alignItems: 'center',
            justifyContent: 'space-between',
            gap: 12,
            paddingTop: 14,
            borderTop: '1px solid var(--mtp-border)',
          }}
        >
          <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
            {paymentMethod && (
              <span
                style={{ display: 'inline-flex', alignItems: 'center', gap: 8, fontSize: 13.5 }}
              >
                <CardGlyph />
                {pmLabel(paymentMethod)}
                <LinkButton onClick={onChangePaymentMethod}>Change</LinkButton>
              </span>
            )}
            <span style={{ fontSize: 12.5, color: 'var(--mtp-text-2)' }}>
              Full access returns once your balance is above zero.
            </span>
          </div>
          <PButton onClick={() => onPay(choice === 'other' ? undefined : choice)}>
            {choice === 'other'
              ? 'Continue'
              : unit
                ? `Buy ${formatCredits(choice, unit)}`
                : `Pay ${formatCurrency(choice, wallet.currency)}`}
          </PButton>
        </div>
      </Card>
    </section>
  )
}

const grantLabel = (grant: CreditGrantView) => {
  const source = grantKindLabel(grant)
  const effective = boundaryDay(grant.effectiveAt)
  return grant.scheduled ? `${source} · Scheduled · from ${effective}` : `${source} · ${effective}`
}

const YourCredit = ({ overview, named }: { overview: LoadedOverview; named: boolean }) => {
  const { wallet } = overview
  const fmt = formatterFor(wallet)
  const short = shortFormatterFor(wallet)
  return (
    <PanelCard
      title={named ? `Your credit · ${walletTitle(wallet)}` : 'Your credit'}
      action={
        <span style={{ fontSize: 12.5, color: 'var(--mtp-text-2)' }}>Used in the order shown</span>
      }
    >
      <ul style={{ listStyle: 'none', margin: 0, padding: 0 }}>
        {overview.grants.map((grant, i) => {
          const left = grant.remainingAfterUsage ?? grant.remaining
          const used = grant.amount > left ? grant.amount - left : 0n
          const pct = clampPct(Number(used), Number(grant.amount))
          return (
            <li
              key={grant.id}
              style={{
                display: 'grid',
                gridTemplateColumns: 'minmax(0, 1.2fr) minmax(0, 1fr) auto',
                alignItems: 'center',
                gap: 16,
                padding: '14px 20px',
                borderTop: i === 0 ? 'none' : '1px solid var(--mtp-border)',
              }}
            >
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2, minWidth: 0 }}>
                <span
                  style={{ display: 'inline-flex', alignItems: 'baseline', gap: 8, fontSize: 13.5 }}
                >
                  <span style={{ fontWeight: 500 }}>{grantLabel(grant)}</span>
                  <UsageFilterTag subscriptionId={grant.scope?.subscriptionId} />
                </span>
                {grant.scope && (
                  <span style={{ fontSize: 12.5, color: 'var(--mtp-text-2)' }}>
                    {grant.scope.usageOnly
                      ? `Usage only · ${grant.scope.paysFor}`
                      : grant.scope.paysFor}
                  </span>
                )}
                {productScopeLabel(grant.applicableProducts) && (
                  <span
                    style={{ fontSize: 12.5, color: 'var(--mtp-text-2)' }}
                    title={grant.applicableProducts.map(product => product.name).join(', ')}
                  >
                    {productScopeLabel(grant.applicableProducts)}
                  </span>
                )}
                <span style={{ fontSize: 12.5, color: 'var(--mtp-text-2)' }}>
                  {grant.expiresAt ? `expires ${boundaryDate(grant.expiresAt)}` : 'never expires'}
                </span>
              </div>
              <Meter
                pct={pct}
                tone="default"
                label={
                  used > 0n
                    ? `${fmt(used)} used of ${fmt(grant.amount)}`
                    : `Nothing used of ${fmt(grant.amount)}`
                }
              />
              <Mono style={{ fontWeight: 600, textAlign: 'right', whiteSpace: 'nowrap' }}>
                {fmt(left)}{' '}
                <span style={{ fontWeight: 500, color: 'var(--mtp-text-2)' }}>
                  / {short(grant.amount)}
                </span>
              </Mono>
            </li>
          )
        })}
      </ul>
    </PanelCard>
  )
}

/** The customer's credit: balances, how to add more, each credit grant and each statement. */
export const BalanceScreen = () => {
  const credit = useCreditOverview()
  const selfService = useSelfService()
  const { overview, goTo } = usePortalNav()
  const [searchParams, setSearchParams] = useSearchParams()
  // `opening` is the customer's money wallet not opened yet: the first quote opens it.
  const [topUp, setTopUp] = useState<{
    walletId: string
    amount?: bigint
    opening?: Wallet
  } | null>(null)

  // Back from the hosted checkout: open the purchase the link names once the wallets are loaded,
  // then drop it from the URL.
  const returnedWalletId = searchParams.get(TOP_UP_RETURN_PARAM)
  const loadedWallets = credit.data?.wallets
  useEffect(() => {
    if (!loadedWallets || !returnedWalletId) return
    const owned = loadedWallets.flatMap(w => (w.wallet ? [w.wallet] : []))
    const returned = returnedWallet(returnedWalletId, owned)
    if (returned && hasPendingTopUp(returned)) setTopUp({ walletId: returned.id })
    const next = new URLSearchParams(searchParams)
    next.delete(TOP_UP_RETURN_PARAM)
    setSearchParams(next, { replace: true })
  }, [loadedWallets, returnedWalletId, searchParams, setSearchParams])

  const loadedForHeader = (loadedWallets ?? []).flatMap(w => (w.wallet ? [w.wallet] : []))
  const header = (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
      <h1 style={{ fontSize: 22, fontWeight: 600, letterSpacing: '-0.02em', margin: 0 }}>Credit</h1>
      <p style={{ fontSize: 13, color: 'var(--mtp-text-2)', margin: 0 }}>
        {creditPaysLine(loadedForHeader)}
      </p>
    </div>
  )

  if (credit.isLoading) {
    return (
      <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
        {header}
        <div style={{ display: 'flex', justifyContent: 'center', padding: '48px 0' }}>
          <Spinner size={20} />
        </div>
      </div>
    )
  }
  if (credit.isError || !credit.data) {
    return (
      <CenterState
        icon={<AlertTriangle size={24} style={{ color: 'var(--mtp-text-3)' }} />}
        title="Credit unavailable"
        hint="We could not load your credit. Please retry."
        action={<PButton onClick={() => void credit.refetch()}>Retry</PButton>}
      />
    )
  }

  const data = credit.data
  const merchantName = overview.invoicingEntityName
  const wallets = data.wallets.filter((w): w is LoadedOverview => !!w.wallet)
  const paymentMethods = overview.paymentMethods
  const defaultPaymentMethodId = overview.customer?.currentPaymentMethodId
  const defaultPaymentMethod =
    paymentMethods.find(method => method.id === defaultPaymentMethodId) ?? paymentMethods[0]
  // A credit unit wallet is bought only at the unit's selling price in the customer's currency.
  const canConfigureWallet = (wallet: Wallet) => !wallet.creditUnit || !!wallet.salePrice
  const topUpEnabled = data.topUpEnabled
  const canTopUp = (wallet: Wallet) => canConfigureWallet(wallet) && topUpEnabled && wallet.canTopUp
  const pausedInline = (wallet: Wallet) =>
    wallet.status === WalletStatus.USAGE_BLOCKED && canTopUp(wallet)
  const topUpWallet = topUp?.opening ?? wallets.find(w => w.wallet.id === topUp?.walletId)?.wallet
  const several = wallets.length > 1
  // Top-ups offered and no wallet yet: credit is bought in the customer's billing currency.
  const opening =
    wallets.length === 0 && topUpEnabled && data.customerCurrency
      ? create(WalletSchema, { id: '', currency: data.customerCurrency, canTopUp: true })
      : undefined

  // The modals keep their place whichever body shows, so a wallet opened mid-purchase keeps them.
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      {header}
      {wallets.length === 0 ? (
        <Fragment key="empty">
          <CenterState
            title="No credit yet"
            hint={
              opening
                ? `Add credit in ${opening.currency} to pay your invoices with it.`
                : `You have no account credit at the moment. Contact ${contactName(merchantName)} to add credit.`
            }
            action={
              opening && (
                <PButton onClick={() => setTopUp({ walletId: '', opening })}>
                  <Plus size={13} /> Add credit
                </PButton>
              )
            }
          />
          {selfService && <RedeemCodeCard />}
        </Fragment>
      ) : (
        <Fragment key="wallets">
          <PortalAlerts
            credit={data}
            onAddCredit={walletId =>
              document
                .getElementById(addCreditAnchor(walletId))
                ?.scrollIntoView({ behavior: 'smooth', block: 'start' })
            }
          />
          {wallets.map(entry => {
            const { wallet } = entry
            const inline = pausedInline(wallet)
            return (
              <div key={wallet.id} style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
                <BalanceCard
                  overview={entry}
                  canTopUp={canTopUp(wallet) && !inline}
                  footer={
                    !inline && (
                      <Slot
                        name="portal.balance.wallet"
                        wallet={wallet}
                        credit={data}
                        standalone={false}
                      />
                    )
                  }
                  onTopUp={() => setTopUp({ walletId: wallet.id })}
                />
                {inline && (
                  <AddCreditCard
                    wallet={wallet}
                    paymentMethod={defaultPaymentMethod}
                    onPay={amount => setTopUp({ walletId: wallet.id, amount })}
                    onChangePaymentMethod={() => goTo('settings')}
                  />
                )}
                {inline && (
                  <Slot name="portal.balance.wallet" wallet={wallet} credit={data} standalone />
                )}
              </div>
            )
          })}
          {wallets
            .filter(entry => entry.grants.length > 0)
            .map(entry => (
              <YourCredit key={entry.wallet.id} overview={entry} named={several} />
            ))}
          {selfService && <RedeemCodeCard />}
          {wallets.map(entry => (
            <WalletStatement key={entry.wallet.id} wallet={entry.wallet} named={several} />
          ))}
        </Fragment>
      )}
      {topUpWallet && data.topUpEnabled ? (
        <TopUpModal
          wallet={topUpWallet}
          paymentMethods={paymentMethods}
          defaultPaymentMethodId={defaultPaymentMethodId}
          initialAmount={topUp?.amount}
          onClose={() => setTopUp(null)}
        />
      ) : null}
      <Slot name="portal.balance.dialogs" credit={data} />
    </div>
  )
}
