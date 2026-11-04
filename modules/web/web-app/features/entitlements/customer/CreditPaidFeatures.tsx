import { timestampDate } from '@bufbuild/protobuf/wkt'
import { createQueryOptions, useTransport } from '@connectrpc/connect-query'
import { Badge, Button, Progress, Skeleton, Tooltip, TooltipContent, TooltipTrigger } from '@md/ui'
import { useQueries } from '@tanstack/react-query'

import { formatWalletAmount, walletTitle } from '@/features/credit/format'
import { LimitTermsBadges, termsOfValue } from '@/features/entitlements/LimitTermsBadges'
import {
  creditExhaustedRule,
  creditSources,
  exhaustedRuleLabel,
  featurePeriodUsage,
  payingGroups,
  refreshesAt,
  sourcesSummary,
  usageOfCredit,
} from '@/features/entitlements/customer/creditPaidFeaturesModel'
import { effectiveValueLabel, isLimitReached } from '@/features/entitlements/utils'
import { formatShortDay } from '@/features/subscriptions/utils/dates'
import { useQuery } from '@/lib/connectrpc'
import { getUpcomingInvoice } from '@/rpc/api/subscriptions/v1/subscriptions-SubscriptionsService_connectquery'
import { listCustomerWallets } from '@/rpc/api/wallets/v1/wallets-WalletsService_connectquery'

import type { CreditPaidFeature, CreditPayer } from '@/rpc/api/entitlements/v1/entitlements_pb'
import type { EffectiveEntitlement } from '@/rpc/api/entitlements/v1/models_pb'
import type { Wallet } from '@/rpc/api/wallets/v1/models_pb'

type Props = {
  customerId: string
  entitlements: EffectiveEntitlement[]
  paid: CreditPaidFeature[]
  onViewCredit: () => void
}

/** The metered features whose usage a wallet pays, each with that wallet's standing. */
export const CreditPaidFeatures = ({ customerId, entitlements, paid, onViewCredit }: Props) => {
  const wallets = useQuery(listCustomerWallets, { customerId })
  const rows = paid.flatMap(feature => {
    const entitlement = entitlements.find(e => e.feature?.id === feature.featureId)
    return entitlement ? [{ feature, entitlement }] : []
  })
  if (rows.length === 0) return null
  return (
    <div>
      <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground mb-1 px-1">
        Paid with credit
      </p>
      <div className="border border-border rounded-lg divide-y divide-border overflow-hidden">
        {rows
          .sort((a, b) =>
            (a.entitlement.feature?.name ?? '').localeCompare(b.entitlement.feature?.name ?? '')
          )
          .map(({ feature, entitlement }) => (
            <div key={feature.featureId} className="flex flex-col gap-2 px-4 py-3 text-sm">
              <div className="flex items-center justify-between gap-4">
                <div className="flex items-center gap-2 min-w-0">
                  <span className="font-medium truncate">{entitlement.feature?.name}</span>
                  <span className="font-mono text-xs text-muted-foreground truncate">
                    {entitlement.feature?.code}
                  </span>
                  <LimitTermsBadges terms={termsOfValue(entitlement.value)} />
                  {isLimitReached(entitlement.value) && (
                    <Badge size="sm" variant="destructive">
                      Limit reached
                    </Badge>
                  )}
                </div>
                {termsOfValue(entitlement.value)?.limit && (
                  <span className="ml-auto text-xs text-muted-foreground tabular-nums">
                    {effectiveValueLabel(entitlement.value)}
                  </span>
                )}
                <Button
                  variant="link"
                  size="sm"
                  className="h-auto p-0 text-xs shrink-0"
                  onClick={onViewCredit}
                >
                  View credit
                </Button>
              </div>
              {wallets.isLoading ? (
                <Skeleton height={36} />
              ) : (
                byWallet(feature.payers).map(([walletId, payers]) => {
                  const wallet = wallets.data?.wallets.find(w => w.id === walletId)
                  return (
                    wallet && (
                      <WalletStanding
                        key={walletId}
                        wallet={wallet}
                        payers={payers}
                        metricId={feature.metricId}
                      />
                    )
                  )
                })
              )}
            </div>
          ))}
      </div>
    </div>
  )
}

const byWallet = (payers: CreditPayer[]) => {
  const wallets = new Map<string, CreditPayer[]>()
  for (const payer of payers)
    wallets.set(payer.walletId, [...(wallets.get(payer.walletId) ?? []), payer])
  return [...wallets]
}

const WalletStanding = ({
  wallet,
  payers,
  metricId,
}: {
  wallet: Wallet
  payers: CreditPayer[]
  metricId: string
}) => {
  const transport = useTransport()
  const subscriptionIds = [...new Set(payers.map(p => p.subscriptionId))]
  const upcoming = useQueries({
    queries: subscriptionIds.map(subscriptionId =>
      createQueryOptions(getUpcomingInvoice, { subscriptionId }, { transport })
    ),
  })
  const groups = payingGroups(wallet.groups, payers)
  const sources = creditSources(groups, wallet)
  const summary = sourcesSummary(sources)
  const credit = groups.reduce((sum, g) => sum + g.available, 0n)
  const balance = wallet.live?.availableAfterUsage ?? wallet.available
  const used = featurePeriodUsage(
    wallet,
    metricId,
    upcoming.map(result => result.data?.invoice)
  )
  const refreshes = refreshesAt(groups)
  const amount = (minor: bigint) => formatWalletAmount(minor, wallet)
  const pct =
    used !== undefined && credit > 0n ? Math.min(100, (Number(used) / Number(credit)) * 100) : 0

  return (
    <div className="flex flex-col gap-1.5 text-xs text-muted-foreground">
      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1">
        <span>
          {walletTitle(wallet)} · Balance{' '}
          <span className="font-medium text-foreground tabular-nums">{amount(balance)}</span>
          {summary && (
            <>
              {' · '}
              {sources.length > 1 ? (
                <Tooltip>
                  <TooltipTrigger className="underline decoration-dotted underline-offset-2">
                    {summary}
                  </TooltipTrigger>
                  <TooltipContent>
                    {sources.map(source => (
                      <p key={source.label} className="flex justify-between gap-4 tabular-nums">
                        <span>{source.label}</span>
                        <span>{amount(source.amount)}</span>
                      </p>
                    ))}
                  </TooltipContent>
                </Tooltip>
              ) : (
                summary
              )}
            </>
          )}
        </span>
        <span>{exhaustedRuleLabel[creditExhaustedRule(wallet, payers)]}</span>
      </div>
      {upcoming.some(result => result.isError) ? (
        <span>Usage this period could not be loaded.</span>
      ) : used === undefined ? (
        <Skeleton height={20} />
      ) : (
        <>
          <Progress value={pct} className="h-1.5" aria-label="Usage against credit this period" />
          <div className="flex flex-wrap justify-between gap-2">
            <span className="tabular-nums">{usageOfCredit(used, credit, wallet)}</span>
            {refreshes && (
              <span>Refreshes {formatShortDay(timestampDate(refreshes).toISOString())}</span>
            )}
          </div>
        </>
      )}
    </div>
  )
}
