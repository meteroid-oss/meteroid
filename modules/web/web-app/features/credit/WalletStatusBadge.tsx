import { Badge } from '@md/ui'

import {
  liveUpdatingDescription,
  liveUpdatingLabel,
  walletHealthDescription,
  walletHealthLabel,
} from '@/features/credit/walletStatus'
import { WalletStatus } from '@/rpc/api/wallets/v1/models_pb'

import type { WalletHealthKey } from '@/features/credit/walletStatus'
import type { BadgeProps } from '@md/ui'

export const walletHealthKey = (status: WalletStatus): WalletHealthKey | undefined => {
  switch (status) {
    case WalletStatus.HEALTHY:
      return 'healthy'
    case WalletStatus.LOW:
      return 'low'
    case WalletStatus.DEPLETED:
      return 'depleted'
    case WalletStatus.USAGE_BLOCKED:
      return 'blocked'
    default:
      return undefined
  }
}

const VARIANT: Record<WalletHealthKey, BadgeProps['variant']> = {
  healthy: 'secondary',
  low: 'warning',
  depleted: 'warning',
  blocked: 'destructive',
}

/** `stale` overrides the status with a neutral "Updating…" badge: a ledger change landed after
 * the live estimate `status` was computed from, so it would otherwise contradict it. */
export const WalletStatusBadge = ({ status, stale }: { status: WalletStatus; stale?: boolean }) => {
  if (stale) {
    return (
      <Badge size="sm" variant="secondary" title={liveUpdatingDescription}>
        {liveUpdatingLabel}
      </Badge>
    )
  }
  const key = walletHealthKey(status)
  // A wallet in good standing carries no badge: only states that need attention are named.
  if (!key || key === 'healthy') return null
  return (
    <Badge size="sm" variant={VARIANT[key]} title={walletHealthDescription[key]}>
      {walletHealthLabel[key]}
    </Badge>
  )
}
