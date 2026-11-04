import { Skeleton } from '@md/ui'

import { WalletBox } from '@/features/credit/WalletBox'
import { creditGroupRows } from '@/features/credit/creditGroups'

import type { ActivityState } from '@/features/credit/CreditGrantsTable'
import type { WalletDenomination } from '@/features/credit/format'
import type { CreditGroup } from '@/rpc/api/wallets/v1/models_pb'

/** The wallet's credit split by kind, one box each: what is left, when it lapses, what rolls over. */
export const CreditGroupsList = ({
  groups,
  denomination,
  usageFilters,
  unbilledByGrant,
  activityState,
}: {
  groups: CreditGroup[]
  denomination: WalletDenomination
  usageFilters: Map<string, string>
  unbilledByGrant: Map<string, bigint>
  /** Without the grants' usage, a group shows its credit on account, and says so. */
  activityState: ActivityState
}) => {
  const rows = creditGroupRows(
    groups,
    denomination,
    usageFilters,
    activityState === 'ready' ? unbilledByGrant : undefined
  )
  // Groups of one kind differ only by what they pay, so that is named then.
  const sharedLabels = new Set(
    rows.filter((row, i) => rows.findIndex(r => r.label === row.label) !== i).map(r => r.label)
  )
  return (
    <>
      {rows.map(row => {
        const lines = [
          sharedLabels.has(row.label) ? row.paysFor : undefined,
          [row.lapse, row.scheduled].filter(Boolean).join(' · '),
          row.rollover,
        ].filter(Boolean)
        return (
          <WalletBox
            key={row.key}
            label={row.usageFilter ? `${row.label} · ${row.usageFilter}` : row.label}
            value={
              activityState === 'loading' ? (
                <Skeleton height={20} width={72} />
              ) : (
                <>
                  {row.amount}
                  {activityState === 'error' && (
                    <span className="ml-1 text-xs font-normal text-muted-foreground">
                      on account
                    </span>
                  )}
                </>
              )
            }
          >
            {lines.map(line => (
              <p key={line}>{line}</p>
            ))}
          </WalletBox>
        )
      })}
    </>
  )
}
