import { Fragment } from 'react'

import { formatMetricQuantity } from '@/features/billing/metrics/units'
import { rollsOver } from '@/features/credit/format'
import { liveUpdatingLabel } from '@/features/credit/walletStatus'
import { CreditSource, WalletStatus } from '@/rpc/portal/wallets/v1/models_pb'

import { usePortalNav } from '../context'
import { compactNumber } from '../format'
import { Card, Eyebrow, LinkButton, Mono } from '../primitives'
import {
  approxAmount,
  boundaryDay,
  creditCount,
  creditsLabel,
  grantKindLabel,
  liveGrants,
  walletAmount,
  walletTitle,
} from '../walletFormat'

import { CreditBar } from './CreditBar'
import { CreditGroups } from './CreditGroups'
import { UsageFilterTag } from './UsageFilterTag'

import type {
  CreditGrantView,
  CreditOutlook,
  Wallet,
  WalletOverview,
} from '@/rpc/portal/wallets/v1/models_pb'
import type { GetCreditOverviewResponse } from '@/rpc/portal/wallets/v1/wallets_pb'
import type { ReactNode } from 'react'

type LoadedOverview = WalletOverview & { wallet: Wallet }

// A paused wallet, or any wallet of a credit-funded plan, still shows: its $0 balance matters.
const shown = (overview: WalletOverview, creditFunded: boolean): overview is LoadedOverview =>
  !!overview.wallet &&
  (creditFunded ||
    overview.wallet.status === WalletStatus.USAGE_BLOCKED ||
    overview.grants.length > 0 ||
    overview.wallet.available !== 0n ||
    (overview.wallet.availableAfterUsage ?? 0n) !== 0n)

const balanceOf = (wallet: Wallet) => wallet.availableAfterUsage ?? wallet.available

const amountFor = (wallet: Wallet) => (minor: bigint) =>
  walletAmount(minor, wallet.currency, wallet.creditUnit)

const approxFor = (wallet: Wallet) => (minor: bigint) =>
  approxAmount(minor, wallet.currency, wallet.creditUnit)

const usageQuantityText = ({ usageQuantity }: WalletOverview) => {
  if (!usageQuantity) return ''
  return usageQuantity.unitLabel
    ? ` · ${formatMetricQuantity(usageQuantity.quantity, usageQuantity.unitLabel, { compact: true })}`
    : ` · ${compactNumber(usageQuantity.quantity)} ${usageQuantity.metricName.toLowerCase()}`
}

const BigFigure = ({ children, size = 30 }: { children: ReactNode; size?: number }) => (
  <Mono style={{ fontSize: size, fontWeight: 600, letterSpacing: '-0.02em' }}>{children}</Mono>
)

const UsageFooter = ({ wallet }: { wallet: Wallet }) => (
  <p style={{ margin: 0, fontSize: 13, color: 'var(--mtp-text-2)' }}>
    {wallet.status === WalletStatus.USAGE_BLOCKED
      ? 'Your balance is 0. Add credits to keep full access.'
      : wallet.pausesAtZero
        ? 'When your balance reaches 0, access to some features may be limited until you add credits. Usage recorded before then is still charged.'
        : 'Usage beyond your credit is added to your next invoice.'}
  </p>
)

/** What the server projects for one subscription's use of the wallet; none while updating. */
const outlookOutcome = (outlook: CreditOutlook, wallet: Wallet): string | undefined => {
  const verdict = outlook.verdict?.verdict
  switch (verdict?.case) {
    case 'covered':
      return `${outlook.partial ? '≈ ' : ''}covered by credit through ${boundaryDay(verdict.value.through)}`
    case 'projectedBeyond':
      return `≈ ${approxFor(wallet)(verdict.value.amount)} beyond your credit by ${boundaryDay(verdict.value.by)}`
    case 'runsOut':
      return `runs out ≈ ${boundaryDay(verdict.value.on)}${verdict.value.pauses ? ', access may then be limited' : ''}`
    case 'beyondCredit':
      return `${amountFor(wallet)(verdict.value.uncovered)} beyond your credit`
    case 'usagePaused':
      return 'balance at 0, access may be limited'
    case 'topUpFailing':
      return 'automatic top-up is not going through'
    case 'automaticTopUp':
      return 'kept topped up automatically'
    default:
      return undefined
  }
}

const Projection = ({ overview }: { overview: LoadedOverview }) => {
  const { wallet, projectedUsage, period, outlooks } = overview
  if (projectedUsage === undefined) return null
  const approx = approxFor(wallet)
  // One outcome per subscription drawing on the wallet, named when there are several.
  const outcomes = outlooks.flatMap(outlook => {
    const text = outlookOutcome(outlook, wallet)
    if (!text) return []
    return [
      <Fragment key={outlook.subscriptionId}>
        {' · '}
        {outlooks.length > 1 && outlook.planName ? (
          <>
            {outlook.planName} <UsageFilterTag subscriptionId={outlook.subscriptionId} />: {text}
          </>
        ) : (
          text
        )}
      </Fragment>,
    ]
  })
  return (
    <span>
      Projected by {period?.end ? boundaryDay(period.end) : 'period end'}{' '}
      <Mono>≈ {approx(projectedUsage)}</Mono>
      {!wallet.creditUnit && ' incl. tax'}
      {outcomes}
    </span>
  )
}

const barLabel = (overview: LoadedOverview) => {
  const { wallet, projectedUsage, period } = overview
  const amount = amountFor(wallet)
  const used = `${amount(overview.periodUsage ?? 0n)} used this period of ${amount(wallet.available)}`
  return projectedUsage !== undefined
    ? `${used}; projected ${approxFor(wallet)(projectedUsage)} by ${period?.end ? boundaryDay(period.end) : 'period end'}`
    : used
}

const linesStyle = {
  display: 'flex',
  flexWrap: 'wrap',
  gap: '6px 22px',
  fontSize: 13,
  color: 'var(--mtp-text-2)',
} as const

const MoneyCreditCard = ({ overview }: { overview: LoadedOverview }) => {
  const { goTo } = usePortalNav()
  const { wallet } = overview
  const amount = amountFor(wallet)
  const periodUsage = overview.periodUsage
  return (
    <Card style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'baseline',
          gap: 12,
        }}
      >
        <Eyebrow>Credit</Eyebrow>
        <LinkButton onClick={() => goTo('balance')}>View credit</LinkButton>
      </div>
      <div style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'baseline', gap: '8px 14px' }}>
        <BigFigure>{wallet.liveStale ? liveUpdatingLabel : amount(balanceOf(wallet))}</BigFigure>
        <span style={{ color: 'var(--mtp-text-2)' }}>balance</span>
      </div>
      <CreditGroups wallet={wallet} />
      {periodUsage !== undefined && (
        <>
          <CreditBar
            base={wallet.available}
            used={periodUsage}
            projected={overview.projectedUsage}
            label={barLabel(overview)}
          />
          <div style={linesStyle}>
            <span>
              Used this period <Mono>{amount(periodUsage)}</Mono> incl. tax
              {usageQuantityText(overview)}
            </span>
            <Projection overview={overview} />
          </div>
        </>
      )}
      <UsageFooter wallet={wallet} />
    </Card>
  )
}

const grantLabel = (grant: CreditGrantView) =>
  grant.source === CreditSource.ROLLED_OVER
    ? grantKindLabel(grant)
    : `${grantKindLabel(grant)} · ${boundaryDay(grant.effectiveAt)}`

const UnitCreditCard = ({ overview }: { overview: LoadedOverview }) => {
  const { goTo } = usePortalNav()
  const { wallet, period } = overview
  const unit = wallet.creditUnit!
  const count = (minor: bigint) => creditCount(minor, unit)
  const periodUsage = overview.periodUsage
  const grants = liveGrants(overview.grants)
  return (
    <Card style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <div
        style={{
          display: 'flex',
          flexWrap: 'wrap',
          justifyContent: 'space-between',
          alignItems: 'baseline',
          gap: 12,
        }}
      >
        <Eyebrow>
          {unit.pluralName}
          {period?.start && period.end
            ? ` · ${boundaryDay(period.start)} – ${boundaryDay(period.end)}`
            : ''}
        </Eyebrow>
        <LinkButton onClick={() => goTo('balance')}>View credit</LinkButton>
      </div>
      <div style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'baseline', gap: '8px 14px' }}>
        <BigFigure>{wallet.liveStale ? liveUpdatingLabel : count(balanceOf(wallet))}</BigFigure>
        {!wallet.liveStale && (
          <span style={{ color: 'var(--mtp-text-2)' }}>
            {creditsLabel(unit)} left of {count(wallet.available)}
          </span>
        )}
      </div>
      {periodUsage !== undefined && (
        <>
          <CreditBar
            base={wallet.available}
            used={periodUsage}
            projected={overview.projectedUsage}
            label={barLabel(overview)}
          />
          <div style={linesStyle}>
            <span>
              Used <Mono>{count(periodUsage)}</Mono>
              {usageQuantityText(overview)}
            </span>
            <Projection overview={overview} />
          </div>
        </>
      )}
      {grants.length > 0 && (
        <ul
          style={{
            listStyle: 'none',
            margin: 0,
            padding: 0,
            borderTop: '1px solid var(--mtp-border)',
          }}
        >
          {grants.map((grant, i) => {
            const left = grant.remainingAfterUsage ?? grant.remaining
            const until = grant.expiresAt
              ? grant.rollover
                ? grant.rollover.maxAmount !== undefined
                  ? `up to ${count(grant.rollover.maxAmount)} ${rollsOver(grant.rollover.maxAmount, unit)} on ${boundaryDay(grant.expiresAt)}`
                  : `all of it rolls over on ${boundaryDay(grant.expiresAt)}`
                : `expires ${boundaryDay(grant.expiresAt)}`
              : 'never expires'
            return (
              <li
                key={grant.id}
                style={{
                  display: 'flex',
                  flexWrap: 'wrap',
                  justifyContent: 'space-between',
                  gap: 8,
                  padding: '12px 0',
                  fontSize: 13,
                  borderTop: i === 0 ? 'none' : '1px solid var(--mtp-border)',
                }}
              >
                <span style={{ display: 'inline-flex', alignItems: 'baseline', gap: 8 }}>
                  {grantLabel(grant)}
                  <UsageFilterTag subscriptionId={grant.scope?.subscriptionId} />
                </span>
                <span style={{ color: 'var(--mtp-text-2)' }}>
                  <Mono style={{ color: 'var(--mtp-text)' }}>{count(left)}</Mono> of{' '}
                  {count(grant.amount)} left · {until}
                </span>
              </li>
            )
          })}
        </ul>
      )}
      <UsageFooter wallet={wallet} />
    </Card>
  )
}

const CompactWalletCard = ({
  overview,
  besideUnits,
}: {
  overview: LoadedOverview
  besideUnits: boolean
}) => {
  const { wallet } = overview
  const appliesTo = wallet.creditUnit
    ? 'Applied to your usage'
    : besideUnits
      ? 'Applied to overage and other charges'
      : 'Applied to your invoices'
  return (
    <Card style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
      <Eyebrow>{walletTitle(wallet)}</Eyebrow>
      <BigFigure size={26}>
        {wallet.liveStale ? liveUpdatingLabel : amountFor(wallet)(balanceOf(wallet))}
      </BigFigure>
      <CreditGroups wallet={wallet} />
      <span style={{ fontSize: 13, color: 'var(--mtp-text-2)' }}>{appliesTo}</span>
    </Card>
  )
}

/**
 * The overview's credit: the credit-unit wallet (or the first wallet) in full, then the other
 * wallets compact, next to the limits card.
 */
export const CreditSummary = ({
  credit,
  limitsCard,
}: {
  credit?: GetCreditOverviewResponse
  limitsCard: ReactNode
}) => {
  const creditFunded = credit?.creditFunded ?? false
  const wallets = (credit?.wallets ?? []).filter((w): w is LoadedOverview => shown(w, creditFunded))
  const primary = wallets.find(w => w.wallet.creditUnit) ?? wallets[0]
  const others = wallets.filter(w => w !== primary)
  const besideUnits = !!primary?.wallet.creditUnit
  return (
    <>
      {primary &&
        (primary.wallet.creditUnit ? (
          <UnitCreditCard overview={primary} />
        ) : (
          <MoneyCreditCard overview={primary} />
        ))}
      {(others.length > 0 || limitsCard) && (
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fit, minmax(280px, 1fr))',
            gap: 16,
          }}
        >
          {others.map(other => (
            <CompactWalletCard key={other.wallet.id} overview={other} besideUnits={besideUnits} />
          ))}
          {limitsCard}
        </div>
      )}
    </>
  )
}
