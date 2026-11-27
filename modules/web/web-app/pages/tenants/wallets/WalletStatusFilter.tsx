import { slotItems } from '@/edition'
import { walletHealthLabel } from '@/features/credit/walletStatus'
import { optionField } from '@/features/filters/FilterBar'
import { WalletStatus } from '@/rpc/api/wallets/v1/models_pb'

import type { WalletListFilter } from '@/edition/types'

/** The `?status=` value of a core or module filter entry. */
export type WalletFilter = string

const CORE_FILTERS: WalletListFilter[] = [
  { value: 'healthy', label: 'Has balance', request: { status: WalletStatus.HEALTHY } },
  { value: 'low', label: walletHealthLabel.low, request: { status: WalletStatus.LOW } },
  {
    value: 'depleted',
    label: walletHealthLabel.depleted,
    request: { status: WalletStatus.DEPLETED },
  },
  {
    value: 'blocked',
    label: walletHealthLabel.blocked,
    request: { status: WalletStatus.USAGE_BLOCKED },
  },
]

const walletFilters = (): WalletListFilter[] => [
  ...CORE_FILTERS,
  ...slotItems('walletList.filters'),
]

export const parseWalletFilter = (value: string): WalletFilter | undefined =>
  walletFilters().find(filter => filter.value === value)?.value

export const walletFilterRequest = (
  filter?: WalletFilter
): { status?: WalletStatus; autoTopUpFailing: boolean } => {
  const request = walletFilters().find(entry => entry.value === filter)?.request
  return { status: request?.status, autoTopUpFailing: request?.autoTopUpFailing ?? false }
}

/** The wallet status as a field of the list's filter bar: one status (core or a module's). */
export const walletStatusField = () =>
  optionField(
    'status',
    'Status',
    walletFilters().map(filter => ({ value: filter.value, label: filter.label })),
    { multi: false }
  )
