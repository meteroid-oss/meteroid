import { CreditGrantStatus } from '@/rpc/api/wallets/v1/models_pb'

import type { CreditGrant, GrantActivity } from '@/rpc/api/wallets/v1/models_pb'

export const USED_UP_PAGE_SIZE = 10

export const isUsedUp = (grant: Pick<CreditGrant, 'status'>) =>
  grant.status === CreditGrantStatus.DEPLETED || grant.status === CreditGrantStatus.EXPIRED

/** Active, but its whole remainder pays usage not yet invoiced: used up once that is invoiced. */
export const isDrained = (
  grant: Pick<CreditGrant, 'status' | 'remaining'>,
  activity?: Pick<GrantActivity, 'unbilled'>
) => grant.status === CreditGrantStatus.ACTIVE && !!activity && grant.remaining <= activity.unbilled

/** What the grant has paid so far, invoiced or not. */
export const grantUsed = (
  activity?: Pick<GrantActivity, 'invoiced' | 'settledAtExpiry' | 'unbilled'>
) => (activity ? activity.invoiced + activity.settledAtExpiry + activity.unbilled : 0n)

/** "Show 3 used-up grants", "Hide 1 used-up grant". */
export const usedUpToggleLabel = (count: number, open: boolean) =>
  `${open ? 'Hide' : 'Show'} ${count} used-up ${count === 1 ? 'grant' : 'grants'}`
